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
