# VCMP

A cross-platform music player for locally stored music, for Windows 10+, Linux and
macOS. Written in Rust with a [Slint](https://slint.dev) UI.

## Status

Early development. The project foundation is in place: the Cargo workspace, a window
with the sidebar, Settings (light/dark/system theme, reduced motion) and About pages,
CI, and the test setup. Library scanning, playback and everything else in the
[v1 specification](docs/specs/2026-10-17-vcmp-music-player.md) are still to be built.

## Build and run

Requires Rust 1.92 or newer. On Linux, install the fontconfig and xkbcommon
development packages first (`libfontconfig1-dev libxkbcommon-dev` on Debian/Ubuntu).

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
- [CLAUDE.md](CLAUDE.md): branches, conventions, localization
- [Original ideas](ideas.md)

## License

VCMP is licensed under either of the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.

Bundled third-party work: the Sora typeface (SIL Open Font License 1.1, see
[assets/fonts/sora/OFL.txt](assets/fonts/sora/OFL.txt)) and Slint, used under the
Slint Royalty-free License 2.0.
