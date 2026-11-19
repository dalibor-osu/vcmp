# Visual and UX improvements

Status: agreed
Date: 2026-11-19
Area: vcmp-app UI (window layout, player bar, now playing, lyrics, sidebar, Settings, About), vcmp-app preferences

## Context of the change

GitHub issue #107 ("Visual/UX improvements") collects seven changes noticed in use.
What exists today, per item:

| # | Today | Where |
|---|---|---|
| 1 | The queue panel slides in and out (`queue-out`, `Theme.duration-medium`). In a wide window (1000 px or more) the page width uses `queue-open`, not the animated value, so pages and now playing jump to their narrower width at once while the panel is still sliding. Narrow windows cover the pages instead. | `crates/vcmp-app/ui/app-window.slint` (`page-width`, `queue-out`); docs/ui.md "Playback" |
| 2 | The bar's track section is laid out with `alignment: start`, so the text column is only as wide as the title and the heart that follows it moves with the title's length. The lossless and scrobble marks sit after the artist name. The progress bar is in the middle section, under the transport buttons. At 720 px the three sections fill the bar exactly. | `crates/vcmp-app/ui/player-bar.slint`; docs/ui.md "Playback" |
| 3 | Lyrics are a mode of the now playing view (`PlayerState.lyrics-shown`, remembered as `ui.lyrics_shown`). A toggle in the view's top right corner swaps the large cover for the lyrics; the bar's Lyrics button opens now playing on the lyrics. Leaving lyrics mode while keeping now playing open is only possible with that corner toggle. | `now-playing.slint`, `lyrics.slint`, `src/player_service/lyrics.rs`; spec 2026-11-07-lyrics-in-now-playing |
| 4 | Synced lyrics glide the current line to the centre with a 16 ms timer step (a third of the remaining distance per tick). Every line has the same size; the current one only changes colour and weight, instantly. | `crates/vcmp-app/ui/lyrics.slint` |
| 5 | Escape closes the now playing view as a whole. Because lyrics are a mode, not a layer, nothing closes "the lyrics" on their own. | `now-playing.slint` (FocusScope) |
| 6 | The sidebar sections are a fixed `nav` array in three groups (Search; Albums to Folders; Settings and About). Rust maps pages to fixed indices. Preferences live in the SQLite `settings` table. | `app-window.slint` (`nav`, `marker-offset`), `src/navigation.rs` (`section`), `src/preferences.rs` |
| 7 | About is its own sidebar section and page: version, update link, licence and credits, and the `AboutSlint` badge that Slint's royalty-free licence requires. | `crates/vcmp-app/ui/pages.slint` (`AboutPage`) |

Several of these overturn earlier owner decisions recorded in docs/DESIGN.md: the
sidebar grouping (2026-10-18), the bar marks (2026-10-28), the heart's place
(2026-11-04, awaiting review) and lyrics as a mode of now playing (2026-11-07).
CLAUDE.md states that the `AboutSlint` widget must stay "on the About page".

## User request

From issue #107, with the session's request to brainstorm it into a spec on a new
worktree and feature branch:

1. Add animation to all views and pages when the queue is sliding in; now the page
   just snaps while the queue slides in.
2. Improve the current song detail in the play bar: move the heart away so its
   position doesn't change with the song title's length, and move the Lossless and
   Scrobble icons above the song progress bar, which fixes their position as well.
3. Once the lyrics view is open, the only way to close it while keeping now playing
   open is the button in the top right corner. Remove that button and make lyrics
   and now playing separate views: clicking the lyrics icon always opens the lyrics
   view (or closes it if it is open), and clicking the now playing icon always
   opens the now playing view (or closes it).
4. Add animations to the lyrics view: sliding, and a larger size for the current
   line.
5. Escape closes the now playing view but not the lyrics view; it should work for
   both.
6. Let the user choose what is shown in the sidebar, from its second section, so
   they can hide Folders for example.
7. Remove "About" and make it a section in Settings.

## Decisions

### Issue #107 bundles 7 UI items. Should one spec cover all of them, or should it be split?

| Option | What it means |
|---|---|
| One spec, all 7 (Recommended) | Matches the issue and the single branch; one review, but a larger PR with several unrelated UI areas |
| Two specs | Now playing/lyrics (3,4,5) separate from the rest (1,2,6,7); cleaner reviews, two branches and PRs |
| One spec, subset now | Spec only some items now and defer the rest; smaller change, issue stays open |

**Answer: One spec, all 7 (Recommended)**

### With lyrics and now playing as separate views, how do they relate when both buttons are used?

| Option | What it means |
|---|---|
| One at a time (Recommended) | Lyrics replaces now playing and vice versa; clicking the other button switches directly, Escape closes whichever is open. Simplest state, the remembered ui.lyrics_shown mode goes away |
| Lyrics stacks on top | Lyrics opens over now playing; Escape or its button closes lyrics first and returns to now playing. Two layers to track, Escape needs two presses |

**Answer: One at a time (Recommended)**

### When the lyrics view is open and the next track has no lyrics, what does it do?

| Option | What it means |
|---|---|
| Stays, says none (Recommended) | The view stays open with the track header and a short 'No lyrics for this track' line; nothing jumps around during an album, but an empty view is possible |
| Falls back to cover | Shows the now playing cover until a track with lyrics comes; never empty, but the view changes on its own between tracks (today's behaviour) |
| Closes itself | The lyrics view closes and returns to the page; never empty, but the user loses the view mid-listening |

**Answer: Stays, says none (Recommended)**

### What does the lyrics view show around the lines?

| Option | What it means |
|---|---|
| Keep today's header (Recommended) | Small 80 px cover, title, artist and the track's More menu above the lines, as lyrics mode looks now; familiar, only the toggle button disappears |
| Lines only | Just the lyrics and a close button; most room for text, but no reminder of what is playing other than the player bar |

**Answer: Keep today's header (Recommended)**

### For 'sliding' in the lyrics view: the code already glides the current line to the centre with a 16 ms timer step. What do you see today?

| Option | What it means |
|---|---|
| Glide feels abrupt (Recommended) | Keep the glide but make it an eased slide on the theme's medium duration, so it matches the rest of the UI's motion; small change |
| It snaps, no glide | Treat it as a bug: find why the glide doesn't run in practice and fix it; needs reproduction first |
| Lines should enter | New lines slide in from below as others move up, a more visible effect; more work and more motion than the rest of the app |

**Answer: Glide feels abrupt (Recommended)**

### How should the current line's larger size look?

| Option | What it means |
|---|---|
| Grow + colour fade (Recommended) | The playing line eases to a larger size and to the accent colour while the previous one eases back; smooth, both on the theme duration |
| Grow only | Only the size eases; the colour and weight still switch instantly as today; less change, slight mismatch between size and colour |

**Answer: Grow + colour fade (Recommended)**

### In a wide window the queue slides in but pages and now playing snap to the narrower width at once. How should they follow the slide?

| Option | What it means |
|---|---|
| Width follows slide (Recommended) | Page width tracks the panel frame by frame, so content narrows in step; smoothest, but the album grid and tables re-lay out every frame, a performance risk to measure |
| Resize after slide | The panel slides over the page at full width, and the page narrows once it settles (widens first on close); no per-frame relayout, but a smaller snap remains |
| Page slides, then fits | The page moves left with the panel without changing width, then takes the new width at the end; cheap and visible motion, content clips briefly at the left |

**Answer: Width follows slide (Recommended)**

### Where does the player bar's heart go so it no longer moves with the title length?

| Option | What it means |
|---|---|
| Fixed at section end (Recommended) | The track section's text column takes the free width and the heart sits at its right edge; long titles elide before it. Stays next to the track, constant position |
| Into right buttons | Heart joins Lyrics, Now playing and Queue on the right; fully fixed, but the 720 px minimum is already exactly full there |
| Before the title | Heart sits between the cover and the title; fixed position, but pushes the text right and reads less like a track action |

**Answer: Before the title**

### Where exactly above the progress bar do the Lossless and Scrobble marks sit?

| Option | What it means |
|---|---|
| Right, above duration (Recommended) | Both marks in a fixed slot at the right end of the transport row, over the total time; quiet corner, no shift when one is hidden |
| Left, above elapsed | Both marks at the left end of the transport row, over the elapsed time; equally fixed, sits closer to the track info |
| Split both ends | Lossless above the elapsed time, Scrobble above the duration; balanced row, but the two related marks are far apart |

**Answer: Left, above elapsed**

### Where does the user choose which sidebar sections (Albums, Artists, Playlists, Labels, Songs, Folders) are shown?

| Option | What it means |
|---|---|
| Settings rows (Recommended) | A 'Sidebar' group in Settings' Appearance with a switch per section; discoverable and keyboard-friendly, one more Settings block |
| Settings + right-click | Same Settings rows, plus a right-click 'Hide' on a sidebar entry; quicker, but a context menu that exists nowhere else in the sidebar |

**Answer: Settings rows (Recommended)**

### When Playlists or Labels is hidden, what happens to its list under the sections?

| Option | What it means |
|---|---|
| List hides too (Recommended) | Hiding Playlists also hides the playlist list (Favourites included); Labels likewise. A hidden section really disappears |
| Only the entry | The section entry goes but its list stays; playlists stay one click away, but the sidebar keeps a list whose section is gone |

**Answer: List hides too (Recommended)**

### The app opens on Albums. What if the user hides Albums?

| Option | What it means |
|---|---|
| First shown section (Recommended) | The app opens on the first section still shown (Search if all are hidden); no new setting |
| Albums can't be hidden | Albums stays always shown as the home page; simplest, but limits the user's choice |

**Answer: First shown section (Recommended)**

### Where does the About content go once it is a section of Settings?

| Option | What it means |
|---|---|
| Last section (Recommended) | An 'About' heading at the bottom of Settings with version, credits and the Made with Slint badge; conventional place, out of the way |
| First section | About at the top of Settings, above the setup row; version and update link seen first, but pushes the settings people change down |

**Answer: Last section (Recommended)**

### When a section is hidden, what about links that lead into it (e.g. the player bar's album or artist name, a search result)?

| Option | What it means |
|---|---|
| Links still work (Recommended) | The page opens as today, with no sidebar entry marked; hiding only tidies the sidebar, nothing becomes unreachable |
| Links disabled | Links into a hidden section stop being links; consistent 'it's gone', but album and artist names lose their navigation |

**Answer: Links still work (Recommended)**

### Today the bar's Lyrics button is disabled while the track has no lyrics. With a lyrics view that can say 'No lyrics', what should the button do?

| Option | What it means |
|---|---|
| Always enabled (Recommended) | The button always opens or closes the lyrics view, which says when there are none; predictable toggle, one click can lead to an empty view |
| Disabled unless open | Disabled for tracks without lyrics, but stays enabled while the lyrics view is open so it can close it; signals 'none' early, the button's state changes per track |

**Answer: Disabled unless open**

## High-level plan

The workstreams are independent except where noted.

**A. Now playing and lyrics as two views (items 3 and 5).** The window shows at most
one of two overlays: now playing (the large cover, as without lyrics today) or
lyrics (today's lyrics-mode header with the 80 px cover, title, artist and the
track's More menu, above the lines). The corner toggle goes. The bar's Now playing
button opens now playing or closes it; its Lyrics button opens lyrics or closes
them; pressing one while the other view is open switches straight to it. Escape
closes whichever view is open. The lyrics view stays open across tracks and says
there are no lyrics for a track that has none. The Lyrics button is disabled for a
track without lyrics, except while the lyrics view is open, so it can always close
it. Delivers the new view model; everything else in the view keeps working
(closing on a click on the background, the album and artist links closing it).

**B. Lyrics motion (item 4), after A.** The glide to the current line becomes an
eased slide on the theme's medium duration instead of the fixed-fraction timer step.
The current line eases to a larger size and to the accent colour while the line it
replaces eases back. Both only apply to synced lyrics; reduced motion makes both
instant, as it does everywhere else.

**C. The queue slide carries the pages (item 1).** In a wide window, the width of
every page, of now playing and of lyrics follows the panel as it slides, in both
directions, instead of jumping at the start. Narrow windows keep covering the pages
as today. Measured in a release build with the album grid, the Songs table and a
long album page before it is accepted.

**D. Player bar track detail (item 2).** The heart moves between the cover and the
title, in a slot of its own that keeps its place whether the track can be liked or
not. The Lossless and Scrobble marks move to the left end of the transport row,
above the elapsed time, in a fixed slot, Lossless first, so neither shifts when the
other is hidden. The title and artist then take the remaining width of the track
section and elide as before. The bar still fits a 720 px window.

**E. Sidebar sections and About in Settings (items 6 and 7), E2 after E1.**
E1: About leaves the sidebar and becomes the last section of Settings, under an
"About" heading, carrying everything the page carries today, the `AboutSlint` badge
included. E2: Settings' Appearance gets a Sidebar group with one switch each for
Albums, Artists, Playlists, Labels, Songs and Folders; Search and Settings are
always shown. A hidden Playlists or Labels also hides its list below the sections.
The choice is remembered. The app opens on the first section still shown, Search if
none is. Links into a hidden section (album and artist names, search results,
Back and Forward) still open its pages, with no sidebar entry marked.

**F. Records, tests and documentation, alongside each workstream.** The owner's
answers go into the owner's design record, replacing the entries they overturn;
the UI documentation describes the new views, the bar layout, the sidebar choice
and About in Settings; the repository instructions' note on `AboutSlint` names its
new place; the manual test list gets the motion checks a test can't judge
(smoothness of the queue slide and the lyrics).
The UI tests that navigate to About, tab through the sidebar or rely on lyrics mode
move to the new structure.

## Architecture decisions

| Decision | Reason it beat the alternative |
|---|---|
| One overlay state with three values (none, now playing, lyrics) replaces `now-playing-open` plus `lyrics-shown`. | Two booleans allow combinations the owner ruled out (both open, lyrics "on" with the view closed). One value makes "one at a time" and "Escape closes whichever is open" fall out of the model. |
| `ui.lyrics_shown` stops being read or written; an existing value is left in the table and ignored. | The mode it remembered no longer exists. Deleting it buys nothing and a later downgrade still finds it. |
| Lyrics motion goes through the theme's duration tokens rather than a hand-rolled timer step. | Reduced motion then works the same way as every other animation (durations go to zero), and the "no durations outside the theme" rule keeps holding. |
| Page width in a wide window is derived from the animated queue position, not from `queue-open`. | It is the smallest change that makes every page, now playing and lyrics follow the slide, because they all take that one width. The cost is a relayout per frame, which C has to measure. |
| The sidebar is driven by the list of shown sections, and navigation, the marker and Back/Forward identify a section by its page, not by its position in a fixed array. | Hiding sections changes positions; a page identity survives that, and allows "no section marked" when a link opens a hidden section's page. The fixed page-to-index mapping in Rust goes with it. |
| The shown sections are one preference in the existing `settings` table, saved with the other appearance preferences. | No schema change; it is a UI preference like zoom or reduced motion. Sections unknown to an older value count as shown, so a future section is not hidden by accident. |
| About is a Settings section, not a page; the page enum, the sidebar entry and its group lose About. | The owner asked for About to be removed; a page kept only for links would be a second way in. The `AboutSlint` badge moves with it and stays visible and keyboard-operable, which a test enforces as before. |

Invariants the tests enforce:

- At most one of now playing and lyrics is open; Escape closes it; each bar button
  opens its view, closes it, or switches from the other.
- The Lyrics button is enabled exactly when the track has lyrics or the lyrics view
  is open.
- The heart and the marks keep their positions when the title changes, when the
  heart is absent and when one mark is hidden.
- Everything stays inside a 720 px window with the queue and each view open.
- Hiding sections removes them (and the Playlists and Labels lists) from the
  sidebar and from the Tab order; the start page is the first shown section.
- The `AboutSlint` badge is reachable in Settings by mouse and keyboard.
- Reduced motion makes the queue slide, the page widths and the lyrics motion
  instant.

## Weaknesses and risks

| Risk | Likelihood | Cost | What reduces it |
|---|---|---|---|
| Per-frame relayout while the queue slides makes the album grid or a long table stutter. | Medium | The animation the issue asks for looks worse than the snap. | Measure with `--release` (`grid_benchmark`, `album_page_benchmark`) before accepting. Open question for the owner if it stutters: fall back to "resize after slide" for the heavy pages only, or for all. |
| The 720 px bar budget is exactly full today. The heart before the title takes width from the track section, and the marks share the transport row with the shuffle and repeat buttons. | High | Something has to shrink or the bar overflows at the minimum width. | Left open for the owner: what gives at 720 px (title width, the transport row's spacing, the marks hidden below some width). `smallest_window_keeps_everything_inside` catches an overflow. |
| A current line that grows can re-wrap onto two lines, which shifts the lines below and the centring the glide aims at. | Medium | Visible jumps in the middle of the glide. | Centre on the grown line's final height, and keep the growth modest; check long lines in a narrow window by hand. |
| A lyrics view that stays open on a track without lyrics shows only a header and one line. | Certain for some albums | An almost empty view. | Accepted by the owner; the disabled-unless-open button signals it per track. |
| Many UI tests use About as their navigation target and tab through a fixed sidebar order. | Certain | A broad test rewrite in the same change. | Move them to Settings or another section together with E1, so the suite never depends on a page that is gone. |
| The eased slide moves the same scroll position as the mouse wheel, the keyboard cursor and click-to-seek; lines can also change faster than the medium duration. | Medium | Fights between the user's scrolling and the glide, or a lagging slide that queues up. | Ease only on line changes, let a new target replace the one in flight, and leave the wheel and keyboard paths as they are. |
| Removing the About page changes the licence obligation's location. | Low | A licence compliance mistake if the badge became hard to find. | The badge stays visible in Settings, with its accessible label and keyboard path; CLAUDE.md is updated to say where. |
| Hiding every content section leaves a sidebar of Search and Settings. | Low | A confusing but working state. | Accepted; the switches in Settings undo it. |
| A user who relied on now playing opening on lyrics (the remembered toggle) loses that. | Low | One extra click. | None needed: the Lyrics button opens lyrics directly. |

Questions deliberately left open:

- What shrinks in the player bar at 720 px (see above); answer after D is laid out
  and the overflow is measurable.
- The exact enlarged size of the current lyric line and the easing curve; decided
  during B against the theme's type scale and checked in screenshots.
- Whether the queue slide is smooth enough with per-frame relayout; answered by the
  measurement in C.
- docs/DESIGN.md's 2026-11-04 heart entry is still awaiting the owner's review; this
  change moves the heart, so that entry is replaced rather than reviewed.

## Out of scope

- Reordering sidebar sections; only showing and hiding them.
- Hiding Search or Settings, or hiding entries of the Playlists and Labels lists
  individually.
- A right-click menu on sidebar entries.
- Any change to the narrow-window queue (it keeps covering the pages).
- New animation for plain (unsynced) lyrics; they keep scrolling as one text.
- Animations other than the queue slide and the lyrics motion, and any change to
  the reduced-motion setting itself.
- Changes to how lyrics are found, read or synced.
- Moving any other Settings content, or restructuring Settings beyond adding the
  Sidebar group and the About section.
