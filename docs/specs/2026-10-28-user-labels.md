# User labels

Status: agreed
Date: 2026-10-28
Area: vcmp-library (schema, labels), vcmp-app (Settings, sidebar, label pages,
track and album menus, filters, track rows, playback for agents), vcmp-mcp (label
and playback tools)

## Context of the change

VCMP describes tracks only through the tags in their files: the nine fields the
tag editor writes, and every other tag stored at scan in `track_tags`
([docs/library.md](../library.md#all-tags)). The owner has no way to mark tracks
with their own words ("chill", "workout") without rewriting music files, and the
library can only be narrowed by the text filter fields on the Albums, Artists and
Songs pages ([docs/ui.md](../ui.md#page-filters)).

What this change builds on:

- **Track ids are stable.** Tracks are never deleted, and a moved file that is
  re-linked keeps its id ([docs/library.md](../library.md#playlists)), so data
  keyed by track id outlives rescans. Playlist entries already rely on this, as
  will the planned Favourites
  ([2026-10-22 spec](2026-10-22-listening-and-library-features.md), not built yet).
- **Track menus** exist in every track list, the Songs table, the queue and the
  now playing view, and act on multi-selections
  ([docs/ui.md](../ui.md#selecting-tracks)). Album grid cards and the small album
  cards on artist pages have no menu. The album page has buttons, which
  `ideas.md` wants replaced by a three-dot overflow menu.
- **The sidebar** order is the owner's decision (docs/DESIGN.md, 2026-10-18):
  Search; Albums, Artists, Playlists, Songs, Folders; Settings, About; with the
  playlist list below the sections.
- **Agents** connect through `crates/vcmp-mcp` ([docs/mcp.md](../mcp.md)). They
  read the library, propose file tag changes for review and edit playlists at
  once through the `Changes` seam to the app. The MCP spec
  ([2026-10-19](2026-10-19-mcp-library-server.md)) deliberately left playback
  control out, so today an agent cannot play anything.
- **Colours** follow rules the tests enforce: no colour literals outside
  `ui/theme.slint`, WCAG AA contrast in both themes.

The owner wants to ask an agent for "some chill music" and have it play tracks
they labelled "chill", and to filter and play the same tracks in the app.

## User request

I would like to add support for user tags (called labels from here on: "Let's call
them labels from now on. Tags should be the file tags."). They are not saved to the
files, only to VCMP's database. I create them in Settings, each being just text and
a colour, and apply them to tracks from the right-click menu. Right-clicking a whole
album does the same and applies them to all the tracks of the album. Labels can be
used for filtering and grouping, and agents can see them through MCP.

Motivation: I open Claude and say something like "Play some chill music". Claude
looks in the library, finds that I have a "chill" label on some songs, and plays
them. Or, in the app, I filter to the "chill" songs and play them.

Later, to address that agents' label edits couldn't be undone: agents' label edits
are handled the same way as tag edits, so I have to approve the changes and can undo
them later the same way.

## Decisions

The questions were asked while the feature was still called "user tags"; they are
kept verbatim. The naming round renamed it to labels.

### When you tag a whole album, what should the tag belong to?

| Option | What it means |
|---|---|
| Its tracks, once (Recommended) | Tags only ever sit on tracks; tagging an album tags the tracks it has now. Simple model and filtering, but a track added to the album later is untagged. |
| The album itself | Albums carry tags their tracks inherit, including tracks added later. Follows the album, but needs rules for removing an inherited tag from one track and for albums that regroup. |
| Both kinds | Track tags and album tags side by side. Most expressive, but two places to look and harder to explain in the UI and to agents. |

**Answer: Its tracks, once**

### How should an agent act on "play some chill music"?

| Option | What it means |
|---|---|
| Play and queue tools (Recommended) | New tools to play a list of tracks now (optionally shuffled) and add tracks to Up next. Does exactly what you asked, but reverses the MCP spec's 'no playback control' and needs a switch-off story. |
| Full playback control | Also pause, resume, skip, volume and reading what plays now. Agents become a remote control; a bigger surface to build and test. |
| Playlist only | No playback tools; the agent builds a 'Chill' playlist and you press Play. Nothing new for agents, but no hands-free play. |

**Answer: Full playback control**

### Can agents change user tags, or only read them?

| Option | What it means |
|---|---|
| Read and apply at once (Recommended) | Agents list tags, find tracks by tag, add or remove tags on tracks and create tags, done at once like playlist edits. Database only, so little risk; a careless agent can mis-tag many tracks. |
| Read only | Agents see tags and tagged tracks but never change them. Safest, but no 'tag my calm albums as chill' help. |
| Apply via review | Tag edits become change sets you accept, like file tag edits. Safe and undoable, but heavy for database-only labels. |

**Answer: Read and apply at once** - revised after the spec was first written: "To
address the agents label edits can't be undone. Let's handle them the same way we
handle tag edits. So user has to approve the changes and can undo them later the
same way." Agents' label edits are therefore reviewed and undoable; see the last
two rounds.

### Where should user tags appear in the app for filtering and grouping? (several can be chosen)

| Option | What it means |
|---|---|
| Tag filter on Songs (Recommended) | Tag chips beside the Songs filter field narrow the table to tagged tracks, then Play or Shuffle plays them. Reuses the table; one page only. |
| Tags in the sidebar (Recommended) | A Tags section listing each tag like playlists, each opening a page of its tracks with Play and Shuffle. This is the grouping; a new page kind to build. |
| Chips on track rows | Coloured dots or chips in album, playlist and Songs rows show a track's tags. See tags at a glance, but busier rows and a cost in the virtualised table. |
| Tag filter on Albums | The album grid narrows to albums holding tagged tracks. Good for browsing by mood, but an album with one tagged track counts the same as a fully tagged one. |

**Answer: all four - Tag filter on Songs, Tags in the sidebar, Chips on track rows, Tag filter on Albums**

### When several tags are picked in a filter, which tracks stay?

| Option | What it means |
|---|---|
| All picked tags (Recommended) | 'chill' + 'rainy' keeps tracks carrying both; each chip narrows further, like the text filter's words. Can't ask for 'chill or rainy' in one go. |
| Any picked tag | 'chill' + 'rainy' keeps tracks carrying either; good for building a broad mix, but adding a chip widens the list, unlike the text filter. |
| Switchable any/all | A toggle beside the chips picks the rule. Covers both, but one more control to explain and keep keyboard-operable. |

**Answer: All picked tags**

### Which albums stay when the album grid is filtered by a tag?

| Option | What it means |
|---|---|
| Any tagged track (Recommended) | An album stays if at least one of its tracks carries the tag. Never hides a tagged track, but an album with one chill track shows as a chill album. |
| Every track tagged | Only albums whose tracks all carry the tag. Precise for albums tagged whole, but one untagged bonus track hides the album. |
| Most tracks tagged | Albums where over half the tracks carry it. A middle ground, but a hidden threshold that's hard to explain. |

**Answer: Any tagged track**

### How does a tag get its colour?

| Option | What it means |
|---|---|
| Fixed palette (Recommended) | Pick one of about 10 named colours defined in theme.slint, each with a light and dark variant that passes AA. Always readable and on-brand; no exact custom shades. |
| Free colour picker | Any RGB value. Full freedom, but breaks the no-literals rule's intent and some picks will be unreadable in one theme. |
| Picker, auto-adjusted | Any hue; VCMP shifts lightness per theme to keep contrast. Freedom with readability, but the shown colour differs from the picked one. |

**Answer (Other): "There should be a color picker as well as predefined palette to
choose from"**

### Where can new tags be created?

| Option | What it means |
|---|---|
| Settings and the menu (Recommended) | Settings manages tags, and the track menu's Tags submenu also has 'New tag...' that creates and applies one. Fast while tagging; a small dialog from the menu. |
| Settings only | Exactly as you described; the menu only lists existing tags. Simpler, but a trip to Settings each time a new mood comes up. |

**Answer: Settings and the menu**

### With any colour allowed, how should a tag's colour be drawn so its name stays readable in both themes?

| Option | What it means |
|---|---|
| Colour dot + plain text (Recommended) | A chip is a coloured dot beside the name in the normal text colour, on a neutral fill. Any colour works in both themes and AA holds; the colour is a smaller accent. |
| Filled chip, auto text | The chip is filled with the colour and VCMP picks black or white text for contrast. Bolder and more colourful, but loud on a pastel UI and a pale pick may vanish on the light theme's background. |
| Filled chip, warn on pick | Filled chips; the picker warns when a colour fails contrast in either theme but lets it be kept. Your choice wins, but unreadable chips are possible. |

**Answer: Colour dot + plain text**

### Where can a whole album be tagged? (several can be chosen)

| Option | What it means |
|---|---|
| Album card right-click (Recommended) | A new right-click / Menu-key menu on album grid cards with a Tags submenu. Tag many albums without opening them; the grid gets its first menu. |
| Album page menu (Recommended) | A Tags submenu in the album page's menu (the planned three-dot overflow from ideas.md). Natural while looking at the album; depends on that menu existing. |
| Artist page albums | The same menu on the small album cards of an artist's page. Consistent everywhere, one more grid to wire. |

**Answer: all three - Album card right-click, Album page menu, Artist page albums**

### When the selected tracks disagree (some have 'chill', some don't), what does clicking 'chill' in the menu do?

| Option | What it means |
|---|---|
| Add to all (Recommended) | A mixed tag shows a partial mark; clicking adds it to every selected track, and a second click removes it from all. Predictable and matches the album case. |
| Remove from all | Clicking a mixed tag clears it from every selected track. Quick cleanup, but a click can silently undo tagging on many tracks. |
| Separate Add / Remove | Two submenus, 'Add tag' and 'Remove tag', no toggles. Never ambiguous, but twice the menu and no way to see current tags there. |

**Answer: Add to all**

### Agents controlling playback is new power. How is it switched on?

| Option | What it means |
|---|---|
| Own switch, off (Recommended) | A second switch under AI agents, 'Let agents control playback', off by default; with it off the playback tools say so. An agent can't start music unasked, at the cost of one more setting. |
| Same AI agents switch | Turning agents on allows playback too. One switch, but any connected agent can start, stop or skip music. |

**Answer: Own switch, off**

### The album page has no menu today. How does its Tags submenu get one?

| Option | What it means |
|---|---|
| Build the overflow menu now (Recommended) | This change adds the three-dot menu from ideas.md (Find cover online, Add to playlist, Edit tags, Tags) and moves the buttons into it. Lands the idea once, but widens this change into album page layout. |
| Right-click the header | A right click (or Menu key) on the album header opens the same menu as a grid card. No layout change, but hard to discover. |
| Wait for the overflow work | The album page gets Tags when the ideas.md overflow menu is built separately; until then, tag from the card or a track selection. |

**Answer: Wait for the overflow work** - so the album page's Tags submenu (chosen in
the album-menu round) arrives with that separate change, not this one.

### How should this be delivered?

| Option | What it means |
|---|---|
| Tags, then playback (Recommended) | One PR for user tags (store, Settings, menus, filters, Tags pages, chips, MCP tag tools), then one for agent playback control. Each reviewable on its own; you can ask an agent for chill music only after the second. |
| One PR | Everything together. The motivating flow works the day it merges, but a large review across four crates. |
| Three PRs | Tag store, Settings and menus; then filters, Tags pages and chips; then MCP tags and playback. Smallest reviews, three merges. |

**Answer: Tags, then playback**

### Should search find tracks by tag name?

| Option | What it means |
|---|---|
| No, tags are filters (Recommended) | Search and the text filters keep matching names only; tags are reached through chips and Tags pages. Searching 'chill' won't surface unrelated titles mixed with tagged tracks. |
| Yes, as their own group | Search shows matching tags as a result group, each opening its Tags page. Discoverable, one more result kind and index. |
| Yes, matches tracks | A tag name counts as a word of each tagged track. 'chill' finds them directly, but mixes with titles containing 'chill'. |

**Answer: No, tags are filters**

### Where do tags sit in the sidebar?

| Option | What it means |
|---|---|
| Tags after Playlists (Recommended) | A Tags section after Playlists with its tags listed below it like the playlist list. Groups the two 'my collections' sections; a long tag list pushes Songs and Folders down. |
| One Tags section only | A single Tags entry after Playlists opening a grid of tags, no per-tag list. Keeps the sidebar short; one more click to reach a tag. |
| Under Songs | Tags listed below the Songs entry, each opening Songs filtered by it. No new page kind, but tag pages and the Songs filter become the same thing. |

**Answer: Tags after Playlists** (so: a Labels section after Playlists)

### The track menu already has 'Edit tags...' for file tags. What should these be called in the UI?

| Option | What it means |
|---|---|
| Labels (Recommended) | 'Labels' in menus, Settings and pages; 'Edit tags...' stays for file tags. No confusion, but differs from the word you used and record labels share it. |
| Tags | 'Tags' submenu next to 'Edit tags...'. Your word and the common one, but two different things called tags side by side. |
| Moods | Fits the 'chill' use case, but suggests mood only, which may not fit tags like 'workout' or 'to sort'. |

**Answer (Other): "Let's call them labels from now on. Tags should be the file
tags."**

### Can one agent change set hold both file tag edits and label edits?

Asked after the owner revised the agents' label edits to go through review.

| Option | What it means |
|---|---|
| Separate kinds (Recommended) | A label change set holds only labels and writes no files; an agent wanting both proposes two sets. Simple accept, conflict and undo rules, but two reviews for one request. |
| Mixed in one set | One set may change tags and labels together, reviewed and undone as a whole. One review per request, but a set can be partly written for files while labels succeed, which complicates results and undo. |

**Answer: Separate kinds**

### When an agent wants a label that doesn't exist yet, how is it created?

| Option | What it means |
|---|---|
| Inside the change set (Recommended) | The set proposes the new label (name, colour) with its tracks; accepting creates it, undo removes it if nothing else uses it. Everything an agent does is reviewed. |
| Created at once | Making an empty label happens immediately, like create_playlist; only applying it is reviewed. Simpler, but unused agent labels can pile up in Settings. |

**Answer: Inside the change set**

## High-level plan

Two pull requests into `main`, in this order.

**1. Labels.** Everything except agent playback:

- *Store.* Labels and the track-label links in the library database, a new schema
  version, with the reads and writes the app and the MCP server share.
- *Settings.* A Labels section: each label with its colour dot, name and number of
  tracks; New label; rename; change colour (a palette of predefined colours plus a
  custom colour picker); delete, which asks first and says how many tracks lose it.
- *Menus.* A Labels submenu in every track menu, listing every label as a toggle
  (on, off or mixed for the selected tracks) and ending in "New label...", which
  creates a label and applies it. The same submenu on a new menu for album grid
  cards and for the album cards on artist pages, acting on the album's tracks.
- *Browsing.* A Labels section in the sidebar after Playlists, with the labels
  listed below it like playlists. Each opens a label page: its tracks, Play and
  Shuffle, played as one queue context named in the queue panel.
- *Filters.* Label chips beside the filter field on the Songs page and the Albums
  page; they combine with the typed filter.
- *Rows.* Label chips (dot and name) on track rows in track lists and a Labels
  column in the Songs table.
- *Agents.* MCP tools to list labels and find tracks by labels, and one to propose
  label changes: labels to add or remove on tracks, and new labels (name, colour)
  to create. The proposal becomes a label change set that the owner reviews,
  accepts or rejects in the existing review dialog, and that `propose_undo` can
  undo, reviewed the same way. Track answers (`get_tracks`, `get_album`,
  playlists) carry each track's labels. The server's instructions explain labels
  against file tags, and that label changes wait for review.
- *Review.* The review dialog and the changes-by-agents list in Settings show label
  sets: each label added or removed with its tracks, grouped as file tag changes
  are, and labels the set would create.
- Docs: library.md (schema, change sets), ui.md, mcp.md, manual-testing.md, and
  DESIGN.md for the sidebar change.

**2. Agent playback.** Depends on 1 only for the motivating flow, not in code.

- A second switch under AI agents in Settings, "Let agents control playback", off
  by default.
- MCP tools to read what plays (state, track, position, volume, Up next), play a
  list of tracks now (in order or shuffled), add tracks next or to the end of Up
  next, pause, resume, skip forward and back, and set the volume. With the switch
  off they answer with an error that names the setting.
- Docs: mcp.md (tools, security table), manual-testing.md with a real client.

The album page's Labels submenu is not in either PR: it comes with the separate
overflow menu work from `ideas.md`.

## Architecture decisions

| Decision | Why it beat the alternative |
|---|---|
| **Labels belong to tracks only.** Labelling an album or a selection writes one link per track it holds now. | One rule for filters, pages and agents; no inheritance to explain or to break when albums regroup. Later tracks of an album stay unlabelled (decided). |
| **Two tables in the library:** labels (id, name unique ignoring case, colour, created) and track-label links (track, label, when added), keyed on the pair and indexed by label. Deleting a label removes its links in the same transaction. The new schema version takes the next free number; the 2026-10-22 spec's features may land first. | Keyed by track id, links survive rescans and re-linked files like playlist entries. Unique names let agents and people refer to a label by name. |
| **Links stay on tracks whose files are gone.** Pages, filters and counts show available tracks; agents see availability. | Same rule as playlist entries; a file back on its drive keeps its labels. |
| **Colour is stored as a palette name or `#rrggbb`.** Palette colours come from `ui/theme.slint` with a light and a dark variant each; custom colours are drawn as stored. | Palette picks follow the theme and stay within the token rule; a custom colour is the owner's data, not a literal in the source, so the no-literals test keeps its meaning. |
| **Colour is only ever a dot.** Chips are a dot plus the name in normal text colour on a neutral fill, everywhere (rows, filters, Settings, sidebar, pages). | Any custom colour keeps names at AA in both themes; the name, not the colour, carries meaning (decided). |
| **Filters keep tracks holding every picked label**, and albums holding at least one track that does. Chips and the typed words combine with AND. | Matches how typed words narrow; an album filter never hides a labelled track (decided). |
| **Search does not match labels.** | Labels are reached through chips and pages; search results stay about names (decided). |
| **Mixed selections: clicking a label adds it to all**; when all have it, the click removes it from all. | Predictable, and the same rule for an album as for a selection (decided). |
| **Agents' label edits are a change set of their own kind**, beside tag edits and playlist deletions: at most 500 tracks a set, recorded through the `Changes` seam, waiting for the owner, discarded at restart, kept 30 days. A set never mixes labels with file tags. | Reviewed and undoable like tag edits, as the owner asked; one kind per set keeps accept, conflict and undo rules simple, since a label set writes no files (decided). |
| **A label set records, per track, which of its labels the track holds before**, and accepting checks that still holds: a track whose labels changed since is a conflict, left alone, and the set is `partly_written`. What each track holds after is recorded. | The same guard as file tags: an agent can't overwrite what the owner changed since it looked. |
| **Undo is a new pending label set** that puts back the `before` state of every track that was changed, and needs the `after` state to still hold. A label the set created is deleted by its undo only when no track holds it any more. | The same review and rules as undoing tag edits; a label the owner has since used elsewhere survives. |
| **A new label proposed in a set is created on accept.** If a label of that name (ignoring case) exists by then, the set uses it and leaves its colour. Deleting a label that a pending set refers to marks those tracks' edits as conflicts. | Names are unique, so the same name means the same label; the owner's deletions win over a waiting proposal. |
| **The owner's own labelling from menus and Settings is immediate and has no history.** | Only agents' changes are reviewed, as with the tag editor against agents' tag edits. |
| **MCP names say "label" everywhere**, never "tag", and the instructions say labels are the owner's own and never in files. | `find_tracks_by_tag` and `list_tag_keys` already mean file tags; an agent must not confuse the two. |
| **Playback reaches agents through a new seam trait in `vcmp-mcp`**, implemented by the app and posted to Slint's event loop like `Changes`. The app checks the playback switch on every call. | The crate stays free of Slint and of the app's player; the switch is enforced where playback lives and takes effect at once. |
| **The playback tools are always listed**, and answer with an error naming the setting while it is off. | The agent can tell the owner what to switch on instead of finding no way to play. |
| **Agent playback is a queue context of its own**, named after the agent in the queue panel. | The owner sees where the music came from, as with album, artist and playlist contexts. |

Tests to enforce: links survive a rescan and a re-linked file; deleting a label
removes its links; names are unique ignoring case; the all-labels filter and the
any-track album filter; mixed toggling; every new string through `@tr()` and the
contrast test for the palette in both themes; label change sets (nothing changes
before accept, conflicts when a track's labels changed, undo, a created label
removed by undo only when unused); MCP label tools and the playback
switch's error through the existing HTTP test harness; the Songs table still
creates only the rows in view with label chips.

## Weaknesses and risks

| Risk | Likelihood | Cost | What reduces it |
|---|---|---|---|
| Labels live only in the database; losing or deleting `library.sqlite3` loses every label, with nothing in the files to rebuild them | Low per year, certain over a long time without backups | All labelling work | Known and accepted by "not in files". A label export or import is left for later (out of scope); docs say where the database is |
| An agent proposes a careless label set over hundreds of tracks, and the owner accepts what is too long to read | Medium | Wrong labels until undone | Review first; the review groups the same label change over many tracks into one row with the count; undo is reviewed like tag undo; 500 tracks a set |
| "Play some chill music" now needs an accept before an agent can label anything new | Certain when labels are missing | A slower first run | Only for changes: finding and playing labelled tracks needs no review |
| The review dialog and history were built for file tags and playlist deletions; a third kind widens their UI and the change-set tables | Medium | More UI and schema work in the first PR | Reuse the per-row grouping; label sets need no cover or file columns |
| An agent starts music or sets a high volume unasked | Low with the switch off by default | Startle; hearing at worst | Own switch, off by default; consider capping how far one call can raise the volume, decided in implementation |
| Slint's `MenuItem` may have no mixed (third) state for the Labels toggles | Medium | The partial mark cannot be drawn natively | Check Slint 1.18's menu API first; fall back to a mark in the item text. Menus are already OS-styled (ui.md, known limits) |
| Slint has no colour picker widget; a custom picker must be built, keyboard operable and readable by assistive technology | High | UI work in the first PR | Palette first; custom colour as a hex field plus a hue and shade area, with the hex field as the keyboard path |
| A custom colour close to the background (pale yellow on the light theme) makes its dot hard to see | Medium | Two labels harder to tell apart at a glance | The name always sits beside the dot; a thin outline on dots is an option for the UI work |
| Chips on rows cost render time and width, most in the virtualised Songs table and narrow windows | Medium | Slower scrolling, truncated titles | Measure with the existing benchmarks before merging; limit chips per row with "+n" |
| Labelling an album labels only its current tracks; a track added later is silently unlabelled | Medium | Surprise | Decided; the album's menu shows the label as mixed then, which hints at it |
| A long label list in the sidebar pushes Songs and Folders down | Low to medium | Navigation | Decided by the owner; the list can scroll or collapse later |
| The album page gets labels only once the overflow menu work lands | Certain until then | One fewer entry point | Album cards and track selections cover it |
| The schema version number may collide with the 2026-10-22 features if PRs run in parallel | Medium | A renumber before merge | Whichever merges second takes the next number |
| "Labels" also means record labels; a later "Record label" field would clash | Low | Wording | Settle wording if such a field ever appears |

Open questions, settled during implementation or by the owner then: the exact chip
layout in rows and the Songs column (UI work with antislop), the palette's colours
(Catppuccin-based per DESIGN.md), the maximum label name length, whether label
filters are remembered between runs, and the volume cap for agents.

## Out of scope

- Writing labels into music files, or reading them from files.
- Labels on albums, artists or playlists, and labels a track inherits from its
  album.
- The album page's Labels submenu (waits for the `ideas.md` overflow menu).
- An any/all switch for label filters; a label filter on the Artists page.
- Search matching label names.
- Smart playlists, label hierarchies, and export or import of labels.
- Undo or history for the owner's own label edits (only agents' sets are
  reviewed and undoable).
- Change sets that mix file tag edits with label edits.
- Agents creating, renaming, recolouring or deleting labels outside a reviewed
  set; deleting or renaming labels stays with the owner in Settings.
- Agent control of seeking, repeat or shuffle modes, the output device, or the
  queue beyond adding tracks; agents managing folders and scans.
- Relating labels to the planned Favourites: they stay separate features.
