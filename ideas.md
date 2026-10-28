# VCMP - A cross platform music player

VCMP should be a cross plaform (Windows 10+, Linux, MacOS) music player that is able to play locally stored music. The aim is a modern look, easy to use, good UX and all the features a music enjoyer needs.

## Technical spec

- Written in Rust
- Supported music file formats: mp3, wav, flac, ogg (more in the future)
- UI framework: to decide
- Database: fully local without a need of a running (SQLite?)
  - There should be a checkbox in settings to turn on/off an automatic update and detection of changes in the source dirs
- Output device selection in settings

## Features

- User can choose multiple source directories of music that will be searched and used for a library
  - This includes subdirectories by default, but there should be a checkbox for that
- Playlist making
  - Each playlist has a name and any amount of songs in it. The thumbnail of a playlist is automatically generated as first 4 album covers that occur in the playlist and can be overriden by a custom image by a user.
- Multiple ways to view the library:
  - Album view: Grid of album covers with artist name and album name
  - Artist view: Grid of artists with their photos and names (photos can be overriden by user or searched automatically on web when user explicitly says so)
  - Playlist view: Grid of user playlists
- Shuffle play
  - 2 settings: true shuffle and less repeats
- LastFM scrobbling
- Light and dark theme

## Future features

- Spotify connection and playback
- Lyrics view
- Custom themes

## Improvements noticed in use (2026-10-28)

- **Change review grouped by file.** The review of an agent's change set is hard to
  navigate. Group the changes per track/file, so each file shows all of its changed
  tags together.
- **Agents can set album covers.** MCP agents can't add or replace album covers yet;
  add a way for them to propose one (reviewed like tag changes).
- **Scroll wheel on the volume slider.** Scrolling while hovering the volume slider
  does nothing; it should change the volume.
- **Format and scrobbling indicators in the player bar.** Small icons showing that the
  playing file is lossless, and that Last.fm scrobbling is on for the track.
- **Zoom setting.** On a big 2K monitor the app looks scaled down. Add a zoom (UI
  scale) option to Settings.
- **Overflow menu on album and artist pages.** The buttons under the cover ("Add to
  playlist…", "Edit tags…") are distracting and rarely used. Replace them with a
  three-dot button next to Play and Shuffle that opens a menu with:
  - albums: Find cover online, Add to playlist, Edit tags;
  - artists: Find photo online, Choose photo.

  Use the freed space for a bigger cover, and make the title and other text a little
  larger.
- **Agent changes in their own dialog.** Many agent change sets and undos clutter the
  Settings page. Show them in a modal opened by a button in Settings instead.
