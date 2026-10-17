# VCMP

A cross-platform music player for locally stored music, for Windows 10+, Linux and
macOS. Written in Rust with a [Slint](https://slint.dev) UI.

## Status

Early development. In place so far:

- the window with Settings (light/dark/system theme, reduced motion, both saved) and
  About pages;
- the library core: source directories, scanning with tag reading, album and artist
  grouping, moved-file re-linking, artwork caching, search and a file watcher;
- the Albums page: a cover grid that stays fast with thousands of albums, and album
  pages with tracks by disc;
- music folders in Settings: add and remove folders, scans in the background with
  progress, and automatic updates when the folders change;
- playback: gapless MP3, FLAC, WAV and Ogg Vorbis, with a player bar (seek, volume and
  mute, shuffle, repeat) and Play, Shuffle and play-from-track on album pages;
- Discord Rich Presence: "Listening to VCMP" with the track and its progress.

The queue panel, the other library views and everything else in the
[v1 specification](docs/specs/2026-10-17-vcmp-music-player.md) are still to be built.

## Build and run

Requires Rust 1.92 or newer. On Linux, install the fontconfig, xkbcommon and ALSA
development packages first (`libfontconfig1-dev libxkbcommon-dev libasound2-dev` on
Debian/Ubuntu).

```bash
cargo run -p vcmp-app
```

## Test

```bash
cargo test --workspace
```

## Documentation

- [v1 specification](docs/specs/2026-10-17-vcmp-music-player.md): scope, decisions, architecture
- [Design direction](docs/DESIGN.md): the owner's visual direction
- [UI foundation](docs/ui.md): how the UI is built, and the rules its tests enforce
- [Library](docs/library.md): storage, scanning, re-linking, artwork and search
- [Playback engine](docs/playback.md): how audio gets from a file to the speakers
- [CLAUDE.md](CLAUDE.md): branches, conventions, localization
- [Original ideas](ideas.md)

## License

VCMP is licensed under either of the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.

Bundled third-party work: the Sora typeface (SIL Open Font License 1.1, see
[assets/fonts/sora/OFL.txt](assets/fonts/sora/OFL.txt)) and Slint, used under the
Slint Royalty-free License 2.0.
