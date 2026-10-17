# VCMP

A cross-platform music player for locally stored music, for Windows 10+, Linux and
macOS. Written in Rust with a [Slint](https://slint.dev) UI.

## Status

Early development. In place so far:

- the window with Settings (light/dark/system theme, reduced motion, both saved) and
  About pages;
- the library core: source directories, scanning with tag reading, album and artist
  grouping, moved-file re-linking, artwork caching, search and a file watcher;
- tag writing in the library core (single and batch, with embedded covers, through a
  verified copy of each file); the tag editor UI is still to come;
- the Albums page: a cover grid that stays fast with thousands of albums, and album
  pages with tracks by disc;
- the Artists page: album artists (or everyone credited) with their albums and the
  tracks they appear on elsewhere;
- search over artists, albums and tracks, by any part of a name (Ctrl+F);
- the Songs page: every track in a sortable table;
- the Folders page: the music folders as a file tree;
- a now playing view with the large cover, opened from the player bar;
- music folders in Settings: add and remove folders, scans in the background with
  progress, and automatic updates when the folders change;
- playback: gapless MP3, FLAC, WAV and Ogg Vorbis, with a player bar (seek, volume and
  mute, shuffle, repeat), Play, Shuffle and play-from-track on album pages, a queue
  panel with Up next, the queue restored after a restart, and a choice of output
  device;
- installers for Windows, macOS and Linux, built and attached to a draft GitHub
  Release when a version tag is pushed (see [Releases](#releases));
- Discord Rich Presence: "Listening to VCMP" with the track, its progress and the
  album cover, looked up on MusicBrainz when the files carry no MusicBrainz ID;
- OS media controls: media keys and the system's media widget (MPRIS on Linux, the
  Windows media flyout, Now Playing on macOS) control playback and show the track.

Playlists, the tag editor's window (writing tags is in the library already), Last.fm
scrobbling, artist photos and the rest of the
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

## Releases

Installers are built by [cargo-packager](https://github.com/crabnebula-dev/cargo-packager)
in the [Release workflow](.github/workflows/release.yml):

| Platform | Installers |
|---|---|
| Windows 10+ (x64) | `.msi` (all users, needs admin rights) and `-setup.exe` (current user, no admin rights) |
| macOS (Apple silicon and Intel) | `.dmg` per architecture |
| Linux (x64, glibc 2.35+) | `.AppImage` and `.deb` |

cargo-packager makes all of these from one configuration
(`[package.metadata.packager]` in [crates/vcmp-app/Cargo.toml](crates/vcmp-app/Cargo.toml)).
cargo-bundle has no AppImage and only an experimental MSI, and cargo-dist makes no
dmg, AppImage or .deb.

The Sora font and the UI icons are compiled into the binary, so the installers only
add the licence texts next to it. The app icon is a placeholder
([packaging/icons/](packaging/icons/)) until VCMP has a logo.

Builds are unsigned for now (signing is an open question in the spec), so Windows
SmartScreen and macOS Gatekeeper warn on first start. On macOS, right-click the app
and choose Open, or run `xattr -dr com.apple.quarantine /Applications/VCMP.app`.

### Cutting a release

1. On a `feature/` branch, set `version` under `[workspace.package]` in the root
   `Cargo.toml` to the new version, run `cargo check` to update `Cargo.lock`, and
   merge the pull request into `main`.
2. Tag the merge commit on `main` and push the tag:

   ```bash
   git switch main && git pull && git tag v0.2.0 && git push origin v0.2.0
   ```

3. The workflow checks that the tag is on `main` and matches the version, builds the
   installers on Windows, macOS and Linux, and creates a **draft** GitHub Release with
   them, a `SHA256SUMS.txt` and generated notes. Review the notes and publish it.

To try the packaging without a release, run the workflow by hand (Actions → Release
→ Run workflow, or `gh workflow run release.yml --ref <branch>`). The installers are
then attached to the run as artifacts. GitHub only offers manual runs for workflows
that are on `main`; before that, a branch can try it by adding itself under
`on: push: branches:` for a commit or two.

To package locally, install cargo-packager (`cargo install cargo-packager --locked`),
then:

```bash
cargo build --release -p vcmp-app
cargo packager --release -p vcmp-app
```

The installers land in `target/packages/`. Each platform makes only its own
installers; the AppImage, WiX and NSIS tools are downloaded on first use. On Linux
distributions newer than the Ubuntu 22.04 the workflow uses, the AppImage step fails
in `strip`; set `NO_STRIP=1` for the `cargo packager` command.

### Integration keys

The Last.fm API key and shared secret and the fanart.tv API key are compiled into
release builds from the repository secrets `VCMP_LASTFM_API_KEY`,
`VCMP_LASTFM_SHARED_SECRET` and `VCMP_FANARTTV_API_KEY` (Settings → Secrets and
variables → Actions), which reach the build as environment variables of the same
names. A secret that is not set is left out, and the feature that needs it stays off.
Local builds read the same variables from the environment. The Discord application ID
is built in (`VCMP_DISCORD_APPLICATION_ID` overrides it).

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
