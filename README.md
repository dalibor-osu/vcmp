# VCMP

A cross-platform music player for locally stored music, for Windows 10+, Linux and
macOS. Written in Rust with a [Slint](https://slint.dev) UI.

> **Platform support:** VCMP is developed and tested mainly on Linux. It builds for
> Windows and macOS, but they get little testing so far, so it is not guaranteed to
> work fully correctly there yet. Reports from those systems are welcome.

## Status

Early development. In place so far:

- the window with Settings (light/dark/system theme, reduced motion that follows the
  OS unless set, both saved) and About pages;
- the library core: source directories, scanning with tag reading, album and artist
  grouping, moved-file re-linking, artwork caching, search and a file watcher;
- tag writing in the library core (single and batch, with embedded covers, through a
  verified copy of each file);
- the Albums page: a cover grid that stays fast with thousands of albums, and album
  pages with tracks by disc; an album without a cover can fetch one from the Cover
  Art Archive, picked from what MusicBrainz finds;
- the Artists page: album artists (or everyone credited) with their albums and the
  tracks they appear on elsewhere;
- search over artists, albums, playlists and tracks, by any part of a name; Ctrl+F
  filters the page shown (Albums, Artists, Songs) or opens Search;
- back and forward through the places shown, with the mouse's side buttons or
  Alt+Left and Alt+Right;
- the Songs page: every track in a sortable table;
- the Folders page: the music folders as a file tree;
- playlists: made from any track's menu or the Playlists page, renamed, reordered
  and deleted, with a picture made from their first four album covers or one of
  your own; entries whose file is gone stay, dimmed, and are skipped;
- a now playing view with the large cover, opened from the player bar;
- music folders in Settings: add and remove folders, scans in the background with
  progress, and automatic updates when the folders change;
- playback: gapless MP3, FLAC, WAV and Ogg Vorbis, with a player bar (seek, volume and
  mute, shuffle, repeat), Play, Shuffle and play-from-track on album pages, a queue
  panel with Up next, the queue restored after a restart, and a choice of output
  device;
- release builds for Windows, macOS and Linux, archived and attached to a draft
  GitHub Release when a version tag is pushed (see [Releases](#releases));
- Discord Rich Presence: "Listening to VCMP" with the track, its progress and the
  album cover, looked up on MusicBrainz when the files carry no MusicBrainz ID;
- OS media controls: media keys and the system's media widget (MPRIS on Linux, the
  Windows media flyout, Now Playing on macOS) control playback and show the track;
- Last.fm scrobbling, in builds made with VCMP's Last.fm API key: connect an account
  in Settings, and plays that count by Last.fm's rules are queued and sent in
  batches, also after time offline; the track playing shows as "now playing";
- an update check: when VCMP starts, it asks GitHub Releases for a newer version and
  links to its release page from the sidebar and the About page (it can be switched
  off in Settings);
- a tag editor for one track, a whole album or the selected tracks, cover included,
  writing only what was changed through a verified copy of each file;
- track lists and a Songs table where a click selects a track and its play button
  (in the track number's place) or a double-click plays it; Ctrl or Shift with a
  click or the arrow keys selects several, to queue them, add them to a playlist or
  edit their tags together; the columns can be resized by dragging their edges;
- artist photos: your own image, or one found on fanart.tv through MusicBrainz when
  you ask (in builds made with VCMP's fanart.tv project key), with an optional
  personal fanart.tv key of your own in Settings; photos from fanart.tv are
  credited under them and on About.

The rest of the [v1 specification](docs/specs/2026-10-17-vcmp-music-player.md) is
still to be built.

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

Each release on [GitHub Releases](https://github.com/dalibor-osu/vcmp/releases) has
an archive per platform, built by the [Release workflow](.github/workflows/release.yml),
with a `SHA256SUMS.txt` to check them against. There are no installers yet: unpack
the archive and run VCMP from where it is.

| Platform | Archive | How to run |
|---|---|---|
| Windows 10+ (x64) | `vcmp-<version>-windows-x64.zip` | Unpack it and run `vcmp.exe`. |
| macOS, Apple silicon | `vcmp-<version>-macos-arm64.zip` | Unpack it, move `VCMP.app` to Applications and open it. |
| macOS, Intel | `vcmp-<version>-macos-x64.zip` | The same. |
| Linux (x64, glibc 2.35+) | `vcmp-<version>-linux-x64.tar.gz` | Unpack it and run `vcmp`. |

Each archive has the licence texts next to the program (in `VCMP.app`'s Resources
on macOS); the Sora font and the UI icons are compiled into the program. The app
icon is a placeholder ([packaging/icons/](packaging/icons/)) until VCMP has a logo.
On Linux, VCMP needs ALSA, fontconfig, xkbcommon and EGL or GL, which desktop
systems have (`libasound2`, `libfontconfig1`, `libxkbcommon0`,
`libxkbcommon-x11-0`, `libegl1` on Debian and Ubuntu).

As the note at the top says, Windows and macOS get little testing so far: VCMP is
developed and tested mainly on Linux, and it is not guaranteed to work fully
correctly on the other two yet.

### Unsigned builds

VCMP isn't code-signed: that takes a paid Apple developer account and, on Windows, a
certificate from a signing service, which the project doesn't have for now. The
programs are the ones the Release workflow built from this repository (compare
`SHA256SUMS.txt`), but Windows and macOS can't tell who made them, so they warn the
first time:

- **Windows:** SmartScreen says "Windows protected your PC". Choose **More info**,
  then **Run anyway**. Windows remembers the choice for that copy.
- **macOS:** opening `VCMP.app` the first time is refused ("Apple could not verify
  …"). Choose **Done**, then open System Settings → Privacy & Security, scroll to
  Security, choose **Open Anyway** next to the message about VCMP, and confirm.
  From macOS 15 on, right-clicking the app and choosing Open no longer skips this.
  Or, from a terminal, clear the download flag before the first start:
  `xattr -dr com.apple.quarantine /Applications/VCMP.app`.
- **Linux:** no warning.

Each new version asks again. Signing, and installers (the cargo-packager
configuration under `[package.metadata.packager]` in
[crates/vcmp-app/Cargo.toml](crates/vcmp-app/Cargo.toml) is kept for them), can come
later.

### Versions

VCMP's versions are dates: `YY.MM.PATCH`, the year and month of the release and a
counter for further releases that month. The first release of October 2026 is
`26.10.0`, a fix later that month `26.10.1`, the next release in November `26.11.0`.
There is no "2.0" to wait for; the version says how new a copy is.

It is a plain three-number version, so Cargo, the `v`-tags, the release workflow's
check and the update check (which compares versions as numbers) take it as it is.
The Windows MSI caps the first two numbers at 255, which a two-digit year and a
month stay under until 2255.

**Pre-releases** carry no suffix: a version to try is published with GitHub's
"Set as a pre-release" box ticked, and promoted to a full release (the box
unticked) once it holds up. The update check skips pre-releases, so only people
who download one get it. A suffix would also break the installers planned for later:
the MSI takes only a numeric one, which Debian then sorts after the release itself. A pre-release that
needs fixing makes way for the next patch number (`26.11.1`).

### Cutting a release

1. On a `feature/` branch, set `version` under `[workspace.package]` in the root
   `Cargo.toml` to the release's version (`YY.MM.PATCH` of the day you will tag; also
   the three VCMP crates' `version` in `crates/vcmp-app/Cargo.toml`), run
   `cargo check` to update `Cargo.lock`, and merge the pull request into `main`.
2. Tag the merge commit on `main` and push the tag:

   ```bash
   git switch main && git pull && git tag v26.10.0 && git push origin v26.10.0
   ```

3. The workflow checks that the tag is on `main` and matches the version, builds
   VCMP on Windows, macOS and Linux, and creates a **draft** GitHub Release with the
   archives, a `SHA256SUMS.txt` and generated notes. Review the notes and publish it,
   ticking "Set as a pre-release" for a version to try first.

Once a release is published, copies of VCMP with the update check on link to it at
their next start. The check takes the highest `vYY.MM.PATCH` tag among the
published releases and skips drafts and pre-releases. It asks GitHub without
signing in, so it finds nothing while the repository is private.

To try the release builds without a release, run the workflow by hand (Actions →
Release → Run workflow, or `gh workflow run release.yml --ref <branch>`). The
archives are then attached to the run as artifacts. GitHub only offers manual runs for workflows
that are on `main`; before that, a branch can try it by adding itself under
`on: push: branches:` for a commit or two.

A release build is `cargo build --release -p vcmp-app`; the program lands in
`target/release/`. For the installers left for later, install cargo-packager
(`cargo install cargo-packager --locked`) and run
`cargo packager --release -p vcmp-app`: they land in `target/packages/`, each
platform making only its own (on macOS `--formats app` makes just `VCMP.app`, as the
workflow does). On Linux distributions newer than the Ubuntu 22.04 the workflow
uses, the AppImage step fails in `strip`; set `NO_STRIP=1` for it.

### Integration keys

The Last.fm API key and shared secret and the fanart.tv API key are compiled into
release builds from the repository secrets `VCMP_LASTFM_API_KEY`,
`VCMP_LASTFM_SHARED_SECRET` and `VCMP_FANARTTV_API_KEY` (Settings → Secrets and
variables → Actions), which reach the build as environment variables of the same
names. The CI workflow passes the two Last.fm secrets as well, so it builds what the
releases ship. A secret that is not set is left out, and the feature that needs it
stays off: without both Last.fm values, Settings has no Last.fm row and nothing is
sent; without the fanart.tv key, artist pages offer no "Find photo online". The
Discord application ID is built in (`VCMP_DISCORD_APPLICATION_ID` overrides it).

Local builds read the same variables from the environment. To try Last.fm, create an
API account at <https://www.last.fm/api/account/create> and build with its key and
shared secret (never commit them):

```bash
VCMP_LASTFM_API_KEY=<api key> VCMP_LASTFM_SHARED_SECRET=<shared secret> cargo run -p vcmp-app
```

To try artist photos, get a project API key at <https://fanart.tv/get-an-api-key/>
and build with it. Users can add their own personal fanart.tv key in Settings,
which goes with the project key, as fanart.tv's terms ask apps to allow:

```bash
VCMP_FANARTTV_API_KEY=<api key> cargo run -p vcmp-app
```

A connected account's session key is kept in the OS keyring (service
`io.github.dalibor-osu.vcmp`), not in the library database, so builds and `VCMP_HOME`
directories on the same machine share it.

## Documentation

- [v1 specification](docs/specs/2026-10-17-vcmp-music-player.md): scope, decisions, architecture
- [Design direction](docs/DESIGN.md): the owner's visual direction
- [UI foundation](docs/ui.md): how the UI is built, and the rules its tests enforce
- [Library](docs/library.md): storage, scanning, re-linking, artwork and search
- [Playback engine](docs/playback.md): how audio gets from a file to the speakers
- [Manual testing](docs/manual-testing.md): what to check by hand before a release
- [CLAUDE.md](CLAUDE.md): branches, conventions, localization
- [Original ideas](ideas.md)

## License

VCMP is licensed under either of the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.

Bundled third-party work: the Sora typeface (SIL Open Font License 1.1, see
[assets/fonts/sora/OFL.txt](assets/fonts/sora/OFL.txt)) and Slint, used under the
Slint Royalty-free License 2.0.
