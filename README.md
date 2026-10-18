# VCMP

A player for the music files on your own drives, for Windows 10 and newer, Linux and
macOS.

> VCMP is developed and tested mainly on Linux. The Windows and macOS versions get
> little testing so far and may not work fully correctly yet.

## What it does

- Sorts the music in your folders into albums and artists by their tags, and
  notices new, changed and moved files by itself.
- Lets you browse by album, artist, song, folder or playlist, and search for any
  part of a name. Ctrl+F filters the page you are on, and the mouse's back and
  forward buttons move through the pages you have visited.
- Plays MP3, FLAC, WAV and Ogg Vorbis without gaps between tracks, with a queue,
  shuffle and repeat. The queue is still there after a restart, and you can choose
  the output device.
- Edits the tags of a track, an album or several selected tracks, covers included.
  It edits a copy of each file and checks it before replacing the original.
- Finds missing album covers online (the Cover Art Archive, through MusicBrainz) and
  artist photos on fanart.tv. You can use your own pictures instead, and give
  playlists one too.
- Works with media keys and your system's media controls, shows what you play on
  Discord and scrobbles to Last.fm.
- Has light and dark themes, and uses less motion when your system asks for it.
- Tells you when a new version is out, unless you turn that off in Settings.

## Download

Get VCMP from [GitHub Releases](https://github.com/dalibor-osu/vcmp/releases).
There is no installer yet: unpack the archive and run VCMP from where it is.

| System | File | Then |
|---|---|---|
| Windows 10 or newer | `vcmp-<version>-windows-x64.zip` | Run `vcmp.exe`. |
| macOS on Apple silicon | `vcmp-<version>-macos-arm64.zip` | Move `VCMP.app` to Applications and open it. |
| macOS on Intel | `vcmp-<version>-macos-x64.zip` | The same. |
| Linux | `vcmp-<version>-linux-x64.tar.gz` | Run `vcmp`. |

Some releases have only the Windows and Linux files. On Linux, VCMP needs Ubuntu
22.04, Debian 12 or another system at least as new (glibc 2.35). Desktop systems
have the libraries it uses; on a minimal Debian or Ubuntu, install `libasound2`,
`libfontconfig1`, `libxkbcommon0`, `libxkbcommon-x11-0` and `libegl1`.

### The first start on Windows and macOS

VCMP isn't code-signed yet, so Windows and macOS can't tell who made it and warn
the first time you open each new version.

- Windows says "Windows protected your PC". Choose More info, then Run anyway.
- macOS refuses to open it. Choose Done, then go to System Settings → Privacy &
  Security and choose Open Anyway next to the message about VCMP. Right-clicking
  the app and choosing Open no longer gets around this on macOS 15 and later. You
  can also run `xattr -dr com.apple.quarantine /Applications/VCMP.app` once before
  the first start.

### Versions

A version is the date of its release: 26.10.0 is the first release of October 2026,
26.10.1 a fix later that month and 26.11.0 the release in November. Releases marked
as pre-releases on GitHub are for trying out, and VCMP's update check doesn't
offer them.

## Build it yourself

You need Rust 1.92 or newer. On Linux, also install the fontconfig, xkbcommon and
ALSA development packages (`libfontconfig1-dev libxkbcommon-dev libasound2-dev` on
Debian and Ubuntu). Then run:

```bash
cargo run -p vcmp-app
```

Your own build has no Last.fm and no online artist photos unless you build it with
API keys for them. [docs/development.md](docs/development.md) explains that, and
how to test VCMP and make a release.

## License

VCMP is licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option. It includes the Sora typeface
([SIL Open Font License 1.1](assets/fonts/sora/OFL.txt)) and uses Slint under the
Slint Royalty-free License 2.0.
