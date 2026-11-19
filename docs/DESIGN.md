# VCMP design direction

Author: the product owner. This file records the owner's answers as given; it is
direction for UI work, applied together with the antislop filter.

Dial: ENERGY 1 / RHYTHM 2 / MOTION 2

## Identity

For me, VCMP should be a modern looking, easy to use music player. Spotify is
probably the closest inspiration, but I don't want it to be a straight up clone of
the UI.

## Personality

Modern, possibly minimal UI look with simplicity and usability as a main focus. Made
by a music enthusiast for music enthusasts.

## Palette

I really like catppuccin and similar pastel color palettes

2026-10-31: VCMP's palettes are its own. Catppuccin is an inspiration for them, not a
palette VCMP implements: the code and documentation don't present it as one or use
its flavour and colour names.

Chosen later from offered options (2026-10-17): a pastel pink as the single neon
accent.

2026-11-05: the accent can be chosen from the label hues; pink stays the default and
there is still one accent at a time (see "Owner decisions", themes).

## Typography

Nothing in my mind now

Chosen later from offered options (2026-10-17): Sora.

## Mood and references

Spotify, musicbee and wora as reference apps, I reall like cyberpunk-ish neon style,
but it should be subtle.

## Motion

I like clean animations, but adding an option for reduced motion would be nice if
user dislikes that.

## App icon

Chosen from offered options (2026-10-17): VCMP has no logo yet, so the installers use
a plain placeholder (a "V" in Sora, the accent pink on the dark base,
`packaging/icons/`) until the real logo exists.

## Owner decisions on flagged patterns

- 2026-10-17, R-01 (neon or pastel palettes): kept. The pastel palette (inspired
  by Catppuccin) and the subtle neon accent are deliberate brand identity; R-01 still
  limits the dose (neon on 1-2 focal elements per screen).

## Owner requests

- 2026-10-18, track lists: a click selects a track and doesn't play it; hovering a
  track replaces its number with a play button that plays it. Detail views spread
  over the whole page width ("half of the page now is empty and it looks weird").
  The column widths in these tables can be changed.
- 2026-10-18, sidebar: the order of the sections stays, grouped as Search, then
  Albums, Artists, Playlists, Songs and Folders, then Settings and About; the
  playlist list stays as it is. Choosing a section always goes to its full view,
  not to the detail it last showed. (2026-10-28: Labels joins after Playlists,
  below. 2026-11-19: About moves into Settings, and the library's sections can be
  hidden, below.)
- 2026-10-18, an album renamed while its page is open: the page goes to the renamed
  album instead of showing it as missing.
- 2026-10-18, selection: in a run of selected tracks, only the first and last
  tracks' outer corners are rounded; the corners inside the run looked wrong. A
  small gap separates the first track from the line under the column headings.
- 2026-10-18, after the first antislop audit (`anti-slop/audit-001-2026-10-18.md`):
  - In a small window the queue covers the whole page area instead of sitting on
    its side; it doesn't cover the sidebar.
  - Errors use a pastel red.
  - No neon: the glow is gone. Pink stays the single accent.
  - The accent's many jobs (finding 3), which buttons are filled (finding 5) and the
    page widths on a full screen (finding 9) are open: the owner isn't sure yet
    about 3 and 5, and has plans for 9.
- 2026-10-20, Settings and the tag editor (part of finding 9): the Settings page
  looked weird with everything on the left and the right side blank. It is wider
  (responsively) and centred in the view. The tag editor and album editing, with
  even more empty room and no sidebar, pop out as a dialog in the middle of the
  screen. Chosen from offered options: Settings' column grows with the window
  between 720 and 960 px, and the cover and photo pickers become the same kind of
  dialog.
- 2026-10-28, after using the app:
  - The buttons under an album's or artist's picture are distracting and seldom
    used. A button with three dots next to Play and Shuffle opens a menu with them:
    find album cover, add to playlist and edit tags for albums; find photo online
    and choose photo for artists. The room goes to a larger cover, and the other
    text is a little bigger.
  - Small icons in the player bar mark a lossless file and Last.fm scrobbling.
    (2026-11-19: moved above the progress bar, below.)
  - On a big 2K monitor the app looks scaled down: Settings gets a zoom.
  - The review of an agent's change is hard to navigate: its changes are grouped
    per song/file.
  - Many agent changes clutter Settings: they move to a dialog opened by a button
    in Settings.
  - The mouse wheel over the volume slider changes the volume.
- 2026-10-28, labels (chosen from offered options,
  `docs/specs/2026-10-28-user-labels.md`): the owner's own names for tracks are
  called labels; "tags" stay the file tags. A Labels section comes after Playlists
  in the sidebar, with the labels listed below the playlists. A label's colour
  comes from a palette or a colour picker ("There should be a color picker as well
  as predefined palette to choose from"), and is drawn as a dot beside the name in
  plain text. Labels show as chips on track rows and as a column in the Songs
  table. An album's three-dot menu has Labels too, for all its tracks.
- 2026-10-30, the Windows title bar (issue #55, chosen from two offered ways): no
  system title bar on Windows. VCMP draws its own minimise, maximise and close
  buttons instead of keeping Windows' bar in the theme's colours. macOS and Linux
  keep their system title bars.
- 2026-11-04, favourites (chosen from offered options,
  `docs/specs/2026-11-04-favourites-and-lastfm-loves.md`): a Favourites playlist is
  pinned first in the sidebar's playlist list and the Playlists grid, shown always,
  even with nothing liked. Its picture is a drawn heart, not covers. Album menus
  have the favourites entry too, for all the album's tracks; a menu offers Remove
  only when every track it acts on is liked.
- 2026-11-05, themes (chosen from offered options, issue #84,
  `docs/specs/2026-11-05-custom-themes.md`, following
  `docs/specs/2026-10-20-lyrics-themes-spotify.md`): the Light / Dark / System
  switch is gone ("I think we should get rid of dark/light switch and just provide
  different themes"). Settings offers one list: four built-in themes, one light and
  three dark that differ in depth and in temperature, with plain names, then the
  user's own theme files (colours only). VCMP follows the OS's light or dark setting
  once, on the first start, and never after. The accent is one of the ten label
  hues, one for every built-in theme; theme files bring their own. Settings shows
  the themes as tiles of their colours with the accents as dots below; the setup
  wizard shows the built-in themes and the accent only.
- 2026-11-07, lyrics (chosen from offered options, issue #85,
  `docs/specs/2026-11-07-lyrics-in-now-playing.md`): lyrics live in the now playing
  view behind a toggle that swaps the large cover for them, the cover going small
  beside the title, the same in wide and narrow windows. The toggle is remembered.
  A track without lyrics shows the view as before. The player bar gets a lyrics
  button that opens now playing on them; it stays in place, disabled, while the
  track has none. "Find lyrics online" is in Settings and in the setup wizard's
  Online services step. (2026-11-19: lyrics become a view of their own, below.)
- 2026-11-19, visual and UX improvements (issue #107, chosen from offered options,
  `docs/specs/2026-11-19-visual-ux-improvements.md`):
  - Lyrics and now playing are separate views, one at a time: the Lyrics button
    always opens the lyrics view (or closes it), the Now playing button always opens
    now playing (or closes it), and pressing one while the other is open switches.
    The toggle in the view's corner and the remembered choice are gone. Escape
    closes either.
  - The lyrics view keeps the small cover, title, artist and menu above the lines.
    On a track without lyrics it stays open and says there are none; the Lyrics
    button is disabled for such a track unless the lyrics view is open.
  - Lyrics animate: the line playing slides to the middle with an eased motion on
    the theme's duration, and grows and fades to the accent while the one before
    eases back.
  - With the queue sliding in or out of a wide window, the pages, now playing and
    lyrics narrow and widen along with it instead of jumping.
  - The player bar's heart sits between the cover and the title, so it no longer
    moves with the title's length. The window's smallest width goes from 720 to
    780 px to make room for it ("Raise minimum to 780 px", chosen over hiding the
    volume slider or eliding the title to nothing).
  - The Lossless and Scrobble marks sit above the progress bar, at the left end of
    the buttons' row above the time played, and hide where the buttons leave them
    no room.
  - Settings' Appearance has a switch per library section (Albums, Artists,
    Playlists, Labels, Songs, Folders) to show it in the sidebar; Search and
    Settings always show. Hiding Playlists or Labels hides its list too; links into
    a hidden section still work; the app opens on the first section shown.
  - About is no longer in the sidebar: it is the last section of Settings.
- 2026-11-19, Settings in sections (chosen from offered options, issue #108,
  `docs/specs/2026-11-19-settings-sections.md`): Settings shows one section at a
  time beside a list of the sections; in a page too narrow for both, a dropdown
  above the section takes the list's place. The rows are regrouped into six
  sections, with About (issue #107) as a seventh at the end: General (Run setup
  again, Check for updates), Library (music folders,
  Update automatically, Library check, Play history, Labels), Playback (Shuffle,
  Output device), Appearance (theme, accent, theme files, zoom, reduce motion,
  and the sidebar's sections from issue #107),
  Online services (lyrics, Discord, Last.fm, the fanart.tv key) and AI agents.
  Settings opens on the section last shown, even after a restart: an exception to
  "choosing a section always goes to its full view" (2026-10-18). Each section is a
  place in back and forward. The setup wizard's agents line and the photo picker's
  refused-key message are links that open their section. A search across the
  sections shows the rows it finds themselves, working; it matches row titles,
  section names and a few keywords per row.

## Awaiting the owner's review

Decisions an agent made while building something the owner asked for. They are not
the owner's until reviewed; keep, change or drop each.

- 2026-10-30, animated pictures (issue #56): an animated GIF, WebP or PNG chosen
  as an album's cover, an artist's photo or a playlist's picture plays on that album's, artist's
  or playlist's page and in the now playing view, at the file's own speed and
  looping. Grids, lists, search, the sidebar, the player bar and the queue show its
  first frame. Reduced motion (the OS's or Settings') shows the first frame
  everywhere. Reason: it is the owner's own content rather than motion of the UI,
  so it plays where the picture is the subject, one at a time; MOTION 2's "nothing
  loops" still holds for the UI itself.
- 2026-10-30, the details of VCMP's own window buttons on Windows (the owner's
  request below): minimise, maximise and close sit in the window's top right corner,
  where Windows puts its own, 46 by 32 px like Windows' own, rather than in the
  sidebar's top row beside "VCMP". Their hover is the surface grey of every other
  button, not Windows' red for Close, since red means an error in VCMP. They are not
  in the Tab order, like Windows' own; Alt+Space opens the window menu instead. The
  window drags by a 16 px strip along its top, clear of every page's controls, and
  by the sidebar down to its first section.
- 2026-11-04, the hearts (issue #78): in track lists the heart sits between the
  title and the time, an outline under the pointer and filled in the accent while
  liked. In the player bar it is a round icon button right of the title and artist,
  filled in the accent while liked (2026-11-19: the owner moved it before the
  title, above). The Favourites picture is the accent heart on
  the surface grey. The favourites entry comes right after Add to queue in track
  menus and card menus, and first in the album page's More menu after the cover
  entries.
- 2026-11-05, the themes' details (issue #84): the two new dark themes are Darker
  (warm brown-greys, base `#1c1917`) and Midnight (neutral greys close to black,
  base `#0e0e10`); Dark is today's cool blue-grey. The names Darker and Midnight are
  the spec's examples. Light accents are the label hues darkened by hand to 4.5:1;
  yellow can't get there and stay yellow, so Light doesn't offer it, and lavender
  turns a vivid blue-violet. Red is offered as an accent although red means an error.
  While a theme file is chosen the accent dots are hidden, with a line saying the file
  brings its own accent. A tile is a small preview of base, mantle, surface, text and
  the accent, with the theme's name below and a ring around the chosen one.
- 2026-11-07, the details of lyrics (issue #85): the lyrics icon is a speech bubble
  with two lines of text. In lyrics mode the small cover is 80 px, the title 26 px,
  and the lines 20 px, centred, at most 720 px wide; the line playing is bold in the
  accent, the others muted, and it stays in the middle of the list, gliding there
  as the mouse wheel glides. The view's toggle sits left of the track's menu in the
  top right corner (2026-11-19: the toggle is gone and the glide eases, above). The bar's lyrics button comes first of its three switches.
  So the new button fits at 720 px, the bar's buttons on the right sit 4 px apart
  instead of 8 px at every width, and at the narrowest the volume slider may shrink
  to 52 px and the playing track's title to 120 px with its cover.
- 2026-11-19, the details of issue #107: the line playing in the lyrics grows from
  20 px to 26 px (`font-title`, the next step of the type scale) and slides over the
  theme's medium duration (220 ms) with an ease-out. The lyrics view on a track
  without lyrics says "No lyrics for this track", muted, in the middle of the view.
  The sidebar switches are one Settings row per section, titled with the section's
  name, under a "Sidebar" row that explains them. The Lossless and Scrobble marks
  sit right-aligned over the 52 px time column, Lossless first, and show from a
  controls width of about 360 px (a window of about 930 px). About in Settings
  reads "VCMP, version ..." under an "About" heading.
- 2026-11-19, the details of Settings' sections (issue #108): the list of sections
  is 200 px wide and uses the sidebar's own entries (a surface fill and bold name on
  the chosen one), with the search field above it; it becomes the dropdown when the
  section would get less than 480 px beside it. The section has no heading of its
  own, as the list names it; the music folders and the labels keep theirs inside
  Library. Search results are grouped under their sections' names, no section is
  marked in the list while searching, choosing a section ends the search, and a
  search is not a place in back and forward. The links read "Open AI agents
  settings" and "Open Online services settings". "Include pre-releases", added with
  self-update meanwhile, sits in General under "Check for updates". About, from
  issue #107, is the last section in the list, and drops its own "About" heading
  like the other sections.
