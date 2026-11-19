# Visual and UX improvements

Spec: docs/specs/2026-11-19-visual-ux-improvements.md
Date: 2026-11-19
Status: implemented
Area: vcmp-app (UI in `ui/`, `src/preferences.rs`, `src/navigation.rs`, `src/player_service/lyrics.rs`), docs

## What was done

- **Two views (items 3, 5):** `PlayerState.overlay` (none, now playing, lyrics) replaces
  `now-playing-open` and `lyrics-shown`; `overlay-open` is what the cover decode and
  the pages' animated pictures read. `NowPlayingView` is both views and keeps the
  last one while fading out; the corner toggle is gone, the lyrics view says "No
  lyrics for this track", and Escape closes either. `ui.lyrics_shown` is no longer
  read or written.
- **Lyrics motion (item 4):** `ui/lyrics.slint` slides with an ease-out over
  `Theme.duration-medium` (a 16 ms timer reading `animation-tick()`), re-aims while the
  line playing grows, and grows the line playing to `font-title` with size and colour
  animated.
- **Queue slide (item 1):** the page width (and the views' width) follows the
  animated `queue-out` (`queue-room` in `ui/app-window.slint`).
- **Player bar (item 2):** the heart has its own slot between the cover and the title;
  the Lossless and Scrobble marks sit at fixed places above the time played and hide
  when the centred buttons leave no room. The owner chose to raise the window's
  minimum width from 720 to 780 px (asked during implementation, recorded in
  docs/DESIGN.md).
- **Sidebar and About (items 6, 7):** `ui/sidebar-sections.slint` (`SidebarSections`,
  `SidebarRows`); the window tracks `section: Page` instead of `selected-index`, the
  marker counts only shown sections, `Page.about` and `AboutPage` are gone and About
  is the last section of `SettingsPage`. `preferences::connect_sidebar` loads and saves
  `ui.sidebar_hidden` and picks the start page, before the history is connected.
- Docs: docs/ui.md, docs/DESIGN.md (owner decisions and agent details awaiting review),
  docs/library.md, docs/manual-testing.md, CLAUDE.md (where `AboutSlint` lives).
- Checks run: `cargo check -p vcmp-app --all-targets`, `cargo fmt --all --check`,
  `cargo clippy -p vcmp-app --all-targets -- -D warnings`, and the test binaries
  `ui_invariants`, `ui_behaviour`, `lyrics`, `preferences` plus the crate's unit tests:
  all pass. At the owner's request the rest of the workspace suite was not run here
  (heavier, run later together with other sessions' work).

## How to test

**Automated coverage.** `cargo test -p vcmp-app --test lyrics --test ui_behaviour
--test preferences` (new: `the_lyrics_and_now_playing_buttons_each_open_their_own_view`,
`escape_closes_the_lyrics_view`, `the_lyrics_view_stays_open_on_a_track_without_lyrics`,
`the_pages_narrow_and_widen_with_the_sliding_queue`,
`hidden_sections_leave_the_sidebar_but_their_pages_still_open`,
`the_sidebar_sections_are_saved_and_the_window_opens_on_the_first_shown`). The whole
suite (`cargo test --workspace`) still has to be run.

**Setup.** `VCMP_HOME=target/try cargo run -p vcmp-app` with a copied music folder
that has one track with an `.lrc` next to it (timed lines) and one without lyrics, in
one album, and one lossless file.

**Steps.**
1. Play the track with the `.lrc`. Press the bar's Lyrics button: the lyrics view
   opens (small cover, title, lines), with no lyrics toggle in its corner.
2. Watch a few lines: each slides smoothly to the middle, grows to a larger size and
   turns to the accent while the one before shrinks back.
3. Press Now playing: it switches to the large cover. Press Lyrics: back to lyrics.
   Press Lyrics again: the view closes. Open it again and press Escape: it closes.
4. Open the lyrics and skip to the track without lyrics: the view stays and says "No
   lyrics for this track"; the Lyrics button stays enabled and closes it; then it is
   greyed.
5. In a 1100 px wide window press the queue button: the album grid narrows along with
   the sliding panel; close it: it widens along with it.
6. Look at the bar: the heart is between the cover and the title and does not move
   between tracks; the lossless mark sits above the time played. Narrow the window to
   its minimum: it stops at 780 px, the marks hide, nothing overlaps.
7. Settings, Appearance, Sidebar: turn off Folders and Playlists: they leave the
   sidebar, and so does the playlists list. Turn off Albums and restart: VCMP opens on
   Artists. The sidebar has no About; Settings ends with an About section and the Made
   with Slint badge.

**Also worth trying.** Reduce motion On: the slide, the growth and the page width all
change at once. Click the player bar's album title with Albums hidden: the album page
opens and no section is marked. Hide all six sections and restart: VCMP opens on
Search.

## What is missing

Nothing - the spec is implemented in full.

## Known problems

- The page relayout per frame during the queue slide is not measured yet with
  `--release` (`grid_benchmark`, `album_page_benchmark`); the spec left the fallback
  open if it stutters.
- The Lossless and Scrobble marks show only from a window of about 930 px wide; below
  that they are hidden (owner's choice), so a narrow window has no lossless or scrobble
  indication at all.
- The window's minimum width is now 780 px instead of 720 px.
- The growth of the line playing and its slide are tuned by reading, not by eye: no
  screenshots were taken (the screenshots example compiles but was not run).
- A row's `changed y` and `changed height` re-aim the slide; on a very long wrapped
  line in a narrow window the centring may still settle a few pixels late.

## Recommended follow-ups

- Run the release benchmarks with the queue sliding over the album grid and a long
  album page, and decide the fallback if it stutters.
- Take the screenshots (`cargo run -p vcmp-app --example screenshots`) for the lyrics,
  player bar and Settings shots and have the owner review the details recorded in
  docs/DESIGN.md under "Awaiting the owner's review".

## Changelog

- Open lyrics and now playing as two separate views from their own buttons; Escape closes either
- Keep the lyrics view open on a track without lyrics, saying there are none
- Slide lyrics smoothly to each new line and enlarge the line playing
- Narrow and widen pages smoothly along with the sliding queue panel
- Move the player bar's heart before the title so it no longer moves with the title's length
- Move the Lossless and Scrobble marks above the progress bar
- Choose which library sections the sidebar shows in Settings
- Move About into Settings
- Raise the smallest window width to 780 px
