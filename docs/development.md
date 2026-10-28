# Development

How to build, test and release VCMP. The conventions for working in the repository
are in [CLAUDE.md](../CLAUDE.md), and the
[v1 specification](specs/2026-10-17-vcmp-music-player.md) sets out what VCMP is
meant to do.

## Build and run

Rust 1.92 or newer. On Linux, install the fontconfig, xkbcommon and ALSA development
packages first (`libfontconfig1-dev libxkbcommon-dev libasound2-dev` on Debian and
Ubuntu, `fontconfig libxkbcommon alsa-lib` on Arch).

```bash
cargo run -p vcmp-app
```

A release build is `cargo build --release -p vcmp-app`, and the program lands in
`target/release/`.

## Test

```bash
cargo test --workspace
```

CI also runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings`. CLAUDE.md, "Test", has the
examples and benchmarks.

## Integration keys

Last.fm needs VCMP's API key and shared secret, and "Find photo online" on artist
pages needs VCMP's fanart.tv project key. They are compiled in from the environment
variables `VCMP_LASTFM_API_KEY`, `VCMP_LASTFM_SHARED_SECRET` and
`VCMP_FANARTTV_API_KEY`. A build without them leaves the feature out: without both
Last.fm values Settings has no Last.fm row and nothing is sent, and without the
fanart.tv key artist pages have no "Find photo online". Never commit the keys.

CI and the Release workflow take them from the repository secrets of the same names
(Settings → Secrets and variables → Actions). CI passes the two Last.fm secrets as
well, so it builds what the releases ship.

For a local build, set them in the shell. To try Last.fm, create an API account at
<https://www.last.fm/api/account/create>; for artist photos, get a project key at
<https://fanart.tv/get-an-api-key/>:

```bash
VCMP_LASTFM_API_KEY=<api key> VCMP_LASTFM_SHARED_SECRET=<shared secret> VCMP_FANARTTV_API_KEY=<api key> cargo run -p vcmp-app
```

People can add their own personal fanart.tv key in Settings, which VCMP sends along
with the project key, as fanart.tv's terms ask apps to allow. The session key of a
connected Last.fm account is kept in the OS keyring (service
`io.github.dalibor-osu.vcmp`), not in the library database, so every build and
`VCMP_HOME` directory on a machine shares it. The Discord application ID is built
in; `VCMP_DISCORD_APPLICATION_ID` overrides it.

## Releases

Each release on GitHub has an archive per platform, built by the
[Release workflow](../.github/workflows/release.yml), and a `SHA256SUMS.txt` to check
them against. An archive holds the program and the licence texts (in `VCMP.app`'s
Resources on macOS); the Sora font and the UI icons are compiled into the program.
The app icon is a placeholder ([packaging/icons/](../packaging/icons/)) until VCMP
has a logo.

The builds aren't code-signed: that takes a paid Apple developer account and, for
Windows, a certificate from a signing service, which the project doesn't have yet.
The README tells people how to open VCMP past the warnings this causes.

### Versions

Versions are dates, `YY.MM.PATCH`: the year and month of the release, and a counter
for further releases that month. The first release of October 2026 is `26.10.0`, a
fix later that month `26.10.1`, the next release in November `26.11.0`. As a plain
three-number version, it suits Cargo, the `v` tags, the Release workflow's check and
the update check, which compares versions as numbers. The Windows MSI caps the first
two numbers at 255, which a two-digit year stays under until 2255.

Pre-releases carry no suffix. A version to try is published with GitHub's "Set as a
pre-release" box ticked and unticked once it holds up; the update check skips
pre-releases. A suffix would break the installers planned for later: the MSI takes
only a numeric one, and Debian would sort it after the release itself. A pre-release
that needs a fix makes way for the next patch number.

### Making a release

1. On a `feature/` branch, set `version` under `[workspace.package]` in the root
   `Cargo.toml` to the release's version, and the `version` of each `vcmp-*`
   dependency with it: four in `crates/vcmp-app/Cargo.toml`, one in
   `crates/vcmp-mcp/Cargo.toml`. Run `cargo check` to update `Cargo.lock` and
   merge the pull request into `main`.
2. Tag the merge commit on `main` and push the tag:

   ```bash
   git switch main && git pull && git tag v26.10.0 && git push origin v26.10.0
   ```

3. The workflow checks that the tag is on `main` and matches the version, builds
   VCMP on Windows, macOS and Linux, and creates a draft GitHub Release with the
   archives, the checksums and generated notes. Review the notes and publish it,
   ticking "Set as a pre-release" for a version to try first.

Copies of VCMP with the update check on link to a published release at their next
start. The check takes the highest `vYY.MM.PATCH` tag among the published releases,
skipping drafts and pre-releases. It asks GitHub without signing in, so it finds
nothing while the repository is private.

To try the release builds without a release, run the workflow by hand (Actions →
Release → Run workflow, or `gh workflow run release.yml --ref <branch>`); the archives
are attached to the run. GitHub offers manual runs only for workflows on `main`, so a
branch can instead add itself under `on: push: branches:` for a commit or two.

### Building the archives locally

While the Release workflow can't run, a Linux machine with Docker builds the Windows
and Linux archives:

```bash
packaging/release-local.sh
```

The script builds in an Ubuntu 22.04 container
([packaging/release-local.Dockerfile](../packaging/release-local.Dockerfile)) like
the workflow, so the Linux program needs glibc 2.35 or newer; built on the host, it
would need the host's glibc. Windows is cross-built there with MinGW-w64 (the
`x86_64-pc-windows-gnu` target), where the workflow uses MSVC. The archives and their
`SHA256SUMS.txt` land in `dist/` and the build in `target/release-local/`. The first
run downloads Ubuntu, Rust and the crates; later runs rebuild only what changed.

Set the [integration keys](#integration-keys) in the shell first: the script passes
the ones that are set into the container. Then create a draft release, check it on
GitHub and publish it, which creates the tag on `main`. Add `--prerelease` for a
pre-release.

```bash
gh release create v26.10.0 dist/vcmp-26.10.0-* dist/SHA256SUMS.txt --draft --target main --title "VCMP 26.10.0" --generate-notes
```

The new tag also starts the Release workflow, which fails to start while Actions
can't run.

### Installers

Installers are left for later. Their cargo-packager configuration stays under
`[package.metadata.packager]` in
[crates/vcmp-app/Cargo.toml](../crates/vcmp-app/Cargo.toml). To build them, install
cargo-packager (`cargo install cargo-packager --locked`) and run
`cargo packager --release -p vcmp-app`. They land in `target/packages/`, each
platform making only its own; on macOS, `--formats app` makes just `VCMP.app`, as
the workflow does. On Linux distributions newer than Ubuntu 22.04, the AppImage step
fails in `strip`; set `NO_STRIP=1` for it.

## Documentation

- [v1 specification](specs/2026-10-17-vcmp-music-player.md): scope, decisions,
  architecture
- [Design direction](DESIGN.md): the owner's visual direction
- [UI](ui.md): how the UI is built, and the rules its tests enforce
- [Library](library.md): storage, scanning, re-linking, artwork and search
- [Playback engine](playback.md): how audio gets from a file to the speakers
- [Manual testing](manual-testing.md): what to check by hand before a release
- [CLAUDE.md](../CLAUDE.md): branches, conventions, localization
- [Original ideas](../ideas.md)
