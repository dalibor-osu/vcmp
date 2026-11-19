# Build times: where they go and how to shorten them

Measured on `feature/ui-polish-116` (version 26.11.0) on 2026-11-19, on Windows 11
with Rust 1.99.0 and the MSVC toolchain. The review changes no code. It was written
because the slow edit-build loop made a rewrite in another language look tempting;
the numbers below show the time goes to the generated UI code and to linking, not to
VCMP's own Rust, so the fixes stay in Rust.

The goal is a shorter loop while developing, above all building and running tests.
Release builds may get slower if that helps.

## Measured

All timings are with a warm `target/` directory, after touching one file without
changing it. That is the best case: a real edit can only take as long or longer.

| Touched | `cargo build -p vcmp-app` | `cargo check -p vcmp-app` |
|---|---|---|
| `crates/vcmp-app/ui/theme.slint` | 27.5 s | 14.2 s |
| `crates/vcmp-app/src/days.rs` (the app's lib) | 27.7 to 35.0 s | 12.9 s (a second run) |
| `crates/vcmp-app/src/main.rs` (the binary only) | 7.7 s | - |
| `crates/vcmp-library/src/lib.rs` | 37.1 s | 13.1 s |

`cargo test -p vcmp-app --no-run` after touching `days.rs`: 87 s, and then it failed
(see "Found on the way").

`cargo build -p vcmp-app --timings` after touching `days.rs`, per unit:

| Unit | Time |
|---|---|
| `vcmp-library` | 1.3 s |
| `vcmp-mcp` | 1.4 s |
| `vcmp-app` lib | 19.5 s |
| `vcmp` binary (mostly linking) | 6.2 s |

Not measured: a cold build after `cargo clean`, and a full `cargo test --workspace`.

## What makes it slow

1. **27 test binaries.** Every file in `crates/vcmp-app/tests/` is its own binary,
   and each links the whole app, Slint and every dependency. With a link of about
   6 s each (the binary above), linking alone can take most of the 87 s. `cargo
   test` also builds every example (3 in `vcmp-app`, more in the other crates) and
   each doctest, which are more links.
2. **Generated Slint code.** `slint_build` turns the 16,000 lines of `.slint` into
   about 220,000 lines (21 MB) of Rust in `target/debug/build/vcmp-app-*/out/app-window.rs`,
   and `slint::include_modules!()` in `crates/vcmp-app/src/lib.rs` compiles it as
   part of the `vcmp-app` lib. Every edit in that lib, and every edit in a crate
   it depends on, recompiles it: that is the 19.5 s. Incremental compilation does
   not save it.
3. **Linking with debug info.** MSVC `link.exe` spends most of its time writing
   debug info (PDB), and it does so once per test binary.

VCMP's own Rust is not the problem: the core crates rebuild in 1 to 2 s each.

## Found on the way: the test build fails in this `target/`

Two runs of `cargo test -p vcmp-app --no-run` failed in different ways:

- rustc crashed with `STATUS_STACK_BUFFER_OVERRUN` (`0xc0000409`) while compiling
  the test target of `crates/vcmp-app/src/main.rs`.
- The next run stopped with "crate `symphonia_codec_aac` required to be available
  in rlib format, but was not found in this form" for `tests/moving_pictures.rs`
  (and `tests/agents.rs`).

The second error usually means a `target/` directory in an inconsistent state,
for example after another tool (an IDE running `cargo check` with other flags)
wrote into it at the same time. Start with `cargo clean`. If the rustc crash comes
back after that, it may be a stack overflow on the large generated code: try again
with `RUST_MIN_STACK=16777216`, and report it to Slint or Rust if that fixes it.

## Proposed fixes

Ordered by what each saves on the test loop for the effort. Measure again after
each one with the commands in "How to measure", so each change keeps its own number.

### 1. One test binary instead of 27

Move the files of `crates/vcmp-app/tests/` into `tests/app/` as modules of a single
`tests/app/main.rs` (`mod agents; mod album_grid; ...; mod common;`).

- Expected gain: one link instead of 27 on every `cargo test`, and one compile of
  `common/`. Probably the largest single saving for the test loop and for agents
  running the suite.
- Pros: tests already initialise the Slint testing backend per test
  (`init_no_event_loop`, one per thread in `lyrics.rs`), and none of them sets
  environment variables or process-wide statics, so running them as threads of one
  process should hold.
- Cons: a failing test no longer runs in a process of its own; a panic that leaves
  shared state behind can affect others. `cargo test --test <name>` becomes
  `cargo test --test app <module>::`. Check that nothing depends on test files
  running in separate processes (the Windows media controls, file watchers).
- The same applies to the other crates' `tests/` folders, at a smaller scale.
- Docs to update if adopted: "Test" in `CLAUDE.md` and `docs/development.md` if
  they name test binaries.

### 2. Build only what the test run needs

No code change, only how tests are started:

```bash
cargo test -p vcmp-app --lib --bins --tests
```

- `--lib --bins --tests` skips the examples (`screenshots`, `grid_benchmark`,
  `album_page_benchmark`, each a full link of the app) and the doctests.
- `-p <crate>` for a change that stays in one core crate:
  `cargo test -p vcmp-library` never compiles Slint or the app.
- Pros: works today; easy to put in `CLAUDE.md` so agents use it too.
- Cons: examples and doctests still need building before a merge, so the full
  `cargo test --workspace` stays in the pre-merge checks.

### 3. Lighter debug info for VCMP's own crates

The dependencies already build with `debug = "line-tables-only"`; VCMP's crates use
full debug info, which `cargo test` inherits through the `test` profile. Adding

```toml
[profile.dev]
debug = "line-tables-only"
```

- Expected gain: smaller PDBs, so a faster link of every test binary.
- Pros: one line. Backtraces and panic locations keep files and lines.
- Cons: the debugger no longer shows local variables in VCMP's code. If the
  debugger is used often, keep full info in `dev` and lower it only for tests
  (`[profile.test] debug = "line-tables-only"`).

### 4. Link with `rust-lld` on Windows

Add `.cargo/config.toml`:

```toml
[target.x86_64-pc-windows-msvc]
linker = "rust-lld.exe"
```

`rust-lld` ships with the Rust toolchain. Linux already links with `lld` by default
since Rust 1.90; macOS keeps the system linker.

- Expected gain: a share of every link, multiplied by the number of test binaries.
- Pros: one file, no code change, easy to undo.
- Cons: a less common setup on Windows; check that the embedded manifest
  (`embed-resource`) still lands in the executables (without it Windows refuses to
  start VCMP, and the tests open the window too, see `build.rs`), and that
  debugging in Rider or Visual Studio still finds the PDB. CI and `release.yml`
  pick the file up as well; release builds may keep `link.exe` if that is safer.

### 5. Move the Slint UI into its own crate

Create `crates/vcmp-ui` holding `ui/`, a `build.rs` with the `slint_build` call, and
a `lib.rs` with only `slint::include_modules!()`. `vcmp-app` depends on it and
re-exports what it needs (`pub use vcmp_ui::*;`), so paths like `vcmp_app::AppWindow`
in the tests keep working.

- Expected gain: an edit in the app's Rust glue or in a core crate no longer
  recompiles the generated code, which takes the 19.5 s lib step down to the glue
  alone, before any test binary is linked. `.slint` edits cost the same as today.
- Pros: works in every build, including tests and CI. Fits the existing split
  (core crates already must not depend on Slint).
- Cons: one more crate, and a one-time move of `ui/`, the Slint parts of `build.rs`
  and the debug-info setting (`with_debug_info` must stay on in debug builds for
  the tests' `ElementHandle` lookups). The Windows manifest part of `build.rs` stays
  in `vcmp-app`. Anything in Rust that reaches into generated items by path needs
  checking.
- Docs to update if adopted: the workspace layout table in `CLAUDE.md`, `docs/ui.md`.

### 6. Windows: keep other tools out of `target/`

- **A separate target directory for the IDE.** If Rider (or rust-analyzer) runs
  `cargo check` into the same `target/` with other flags, each side invalidates
  the other's artifacts and waits on the build lock. Point the IDE at its own
  directory (for rust-analyzer, `rust-analyzer.cargo.targetDir`). This may also be
  behind the broken test build above.
- **Dev Drive or a Defender exclusion for the repository.** Windows 11's Dev Drive
  (ReFS, with Defender in performance mode) is Microsoft's recommended place for
  source and build output; real-time scanning of thousands of `.rlib` and `.pdb`
  files is a known cost of Rust builds on Windows. Changing Defender settings is a
  security decision: weigh it before doing it.
- Pros: no change to the repository.
- Cons: per-machine setup; Dev Drive means moving the checkout.

### 7. Slint live preview, for UI work only

Slint 1.13 added a live preview for Rust: with the `live-preview` feature the Slint
compiler replaces the generated code with stubs that have the same API and load the
`.slint` files at run time, and the running app reloads them when they change on
disk.

```bash
SLINT_LIVE_PREVIEW=1 cargo run -p vcmp-app --features slint/live-preview
```

- It does not help tests: they need the compiled UI with debug info for
  `ElementHandle`. It helps when tuning the UI by eye, where a `.slint` edit now
  costs a 27.5 s rebuild and a restart.
- Pros: no code change. Slint's docs say not to put the feature in `[features]`,
  so normal builds, tests and releases stay as they are.
- Cons: development only. Renaming a property or callback that Rust uses still needs
  a rebuild and restart. A `.slint` syntax error keeps the previous version on
  screen. The UI is interpreted, so don't judge performance in this mode (the album
  grid budget in `docs/ui.md`). Untested with VCMP's build script and the
  `unstable-winit-030` hook: check that keyboard shortcuts and the title bar work.
- Docs to update if adopted: "Run locally" in `CLAUDE.md`, `docs/ui.md`.

### Considered and left out

- **The Cranelift backend** (`-Zcodegen-backend=cranelift`): needs nightly, and VCMP
  pins stable Rust 1.92 as its minimum.
- **Splitting the UI into several Slint compile units:** Slint compiles one root
  component tree from `app-window.slint`; splitting it means separate windows, not a
  build setting.
- **A rewrite in another language:** the slow parts above are not caused by Rust;
  the core crates compile in about a second each.

## How to measure

Use the same commands for each fix, so the numbers stay comparable with the tables
above. The test build is the main number to watch:

```bash
touch crates/vcmp-app/src/days.rs && time cargo test -p vcmp-app --no-run
```

```bash
touch crates/vcmp-library/src/lib.rs && time cargo test --workspace --no-run
```

```bash
touch crates/vcmp-app/src/days.rs && time cargo build -p vcmp-app
```

```bash
touch crates/vcmp-app/ui/theme.slint && time cargo build -p vcmp-app
```

```bash
cargo build -p vcmp-app --timings
```

The last one writes an HTML report to `target/cargo-timings/` with the time per
crate. Run `cargo clean` once before the first baseline, then run each command twice
and take the second result, so a stale `target/` does not count against a fix.
