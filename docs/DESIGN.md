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

Chosen later from offered options (2026-10-17): Catppuccin Pink as the single neon
accent.

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
a plain placeholder (a "V" in Sora, Catppuccin Pink on the dark base,
`packaging/icons/`) until the real logo exists.

## Owner decisions on flagged patterns

- 2026-10-17, R-01 (neon or pastel palettes): kept. The Catppuccin-style pastel
  palette and the subtle neon accent are deliberate brand identity; R-01 still
  limits the dose (neon on 1-2 focal elements per screen).

## Owner requests

- 2026-10-18, track lists: a click selects a track and doesn't play it; hovering a
  track replaces its number with a play button that plays it. Detail views spread
  over the whole page width ("half of the page now is empty and it looks weird").
  The column widths in these tables can be changed.
- 2026-10-18, sidebar: the order of the sections stays, grouped as Search, then
  Albums, Artists, Playlists, Songs and Folders, then Settings and About; the
  playlist list stays as it is. Choosing a section always goes to its full view,
  not to the detail it last showed.
- 2026-10-18, an album renamed while its page is open: the page goes to the renamed
  album instead of showing it as missing.
- 2026-10-18, selection: in a run of selected tracks, only the first and last
  tracks' outer corners are rounded; the corners inside the run looked wrong. A
  small gap separates the first track from the line under the column headings.
- 2026-10-18, after the first antislop audit (`anti-slop/audit-001-2026-10-18.md`):
  - In a small window the queue covers the whole page area instead of sitting on
    its side; it doesn't cover the sidebar.
  - Errors use Catppuccin Red.
  - No neon: the glow is gone. Pink stays the single accent.
  - The accent's many jobs (finding 3), which buttons are filled (finding 5) and the
    page widths on a full screen (finding 9) are open: the owner isn't sure yet
    about 3 and 5, and has plans for 9.
