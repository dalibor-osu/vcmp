# In-app self-update

Spec: docs/specs/2026-11-19-in-app-self-update.md
Date: 2026-11-19
Status: implemented
Area: vcmp-integrations (updates, self_update, update_signing example), vcmp-app
(updates, main, update dialog), release workflow, packaging/release-local.sh, docs

## What was done

- Signed releases: `packaging/update-signing.pub` (a placeholder until the key is
  made), `crates/vcmp-integrations/examples/update_signing.rs` (`generate`, `sign`),
  a signing step in `.github/workflows/release.yml` (secrets
  `VCMP_UPDATE_SIGNING_KEY`, `VCMP_UPDATE_SIGNING_PASSWORD`) and in
  `packaging/release-local.sh` (`VCMP_UPDATE_SIGNING_KEY_FILE`). Unsigned releases
  stay possible and only get the link.
- The check (`vcmp_integrations::updates`) returns the release's assets and its
  pre-release flag, and counts flagged pre-releases when asked.
- `vcmp_integrations::self_update`: `plan` (platform, debug build, key, signed
  assets, writable folder), download with progress and cancel, minisign and SHA-256
  check, zip/tar.gz extraction, the swap through `self-replace` with a
  `vcmp.old(.exe)` copy, and `HandOver::run` with roll-back.
- App: `UpdateDialog` in `ui/updates.slint`, the note now opens it, an "Include
  pre-releases" switch (`integrations.update_prereleases`), and `main` hands over
  after the window closes and reads `--update-started` / `--update-failed`.
- Docs: README "Updating", docs/development.md "Signing updates", docs/ui.md
  "Self-update", docs/manual-testing.md, CLAUDE.md stack table.
- Checks run: `cargo fmt --all --check`; `cargo test -p vcmp-integrations --lib`
  for `updates` and `self_update` (26 passed); `cargo test -p vcmp-app --test
  updates --test setup --test ui_invariants` (29 passed); `cargo clippy -p
  vcmp-integrations --all-targets -D warnings` (clean); the `vcmp` binary and the
  screenshots example build without warnings; the update shots were rendered and
  looked at. The signing example was tried with a throwaway key (sign, verify,
  wrong password refused). Not run: `cargo test --workspace`, clippy on vcmp-app and
  the whole workspace, a release build.

## How to test

**Automated coverage.** Unit tests in `crates/vcmp-integrations/src/self_update.rs`
and `src/updates.rs`, UI tests in `crates/vcmp-app/tests/updates.rs`; no E2E suite
drives a real install (it needs a signed release and a release build).

**Setup.**
1. Make a key: `cargo run -p vcmp-integrations --example update_signing --
   generate <key file>`; this writes `packaging/update-signing.pub`.
2. Build two versions with that key: bump `version` in the root `Cargo.toml` (and
   the `vcmp-*` dependency versions, docs/development.md "Making a release"), build
   `cargo build --release -p vcmp-app` for each, and make archives laid out as the
   Release workflow makes them.
3. Publish the newer one as a GitHub release (a fork, or a draft made public for the
   test) with `SHA256SUMS.txt` signed by `update_signing sign`.
4. Unpack the older one into a folder you can write to.

**Steps.**
1. Start the older `vcmp(.exe)`: within seconds the sidebar shows "VCMP x is
   available".
2. Activate it: the dialog shows the version, "You have VCMP y...", Release notes,
   "Install and restart" and "Not now", with the focus on "Not now".
3. Play a track, then choose "Install and restart": a progress bar fills and Cancel
   is the only enabled button.
4. The window closes and the newer version opens within a few seconds with the
   queue restored, paused at the saved position.
5. The folder holds only the program and the licence texts; no `vcmp.old(.exe)`,
   no `.vcmp-update-*` file.

**Also worth trying.**
- Cancel during the download: the dialog returns to its offer; nothing changed on
  disk.
- A release whose archive doesn't match `SHA256SUMS.txt`, or signed with another
  key: "The download doesn't carry VCMP's signature", Try again, nothing replaced.
- A newer version that exits at once: after up to 60 s the old version opens with
  "The update didn't start" and its program is back.
- A read-only folder (Program Files, `chmod a-w`): the dialog explains and offers
  "Open release page".
- A debug build (`cargo run`): "This is a development build".
- A release flagged as a pre-release: shown only while "Include pre-releases" is on,
  marked "A pre-release, published for testing.".

## What is missing

- No signing key exists yet: until the owner runs `update_signing generate`, commits
  `packaging/update-signing.pub` and sets both repository secrets, every build says
  it can't check signatures and only links to the release page.
- Never exercised end to end: a real download from GitHub, the real
  `self_replace` swap (especially on Windows), a real spawn and hand-over, the
  workflow's signing step (it builds vcmp-integrations on a bare ubuntu-latest), and
  the signing step in `release-local.sh` (which needs Rust on the host).

## Known problems

- The confirmation marker sits in the program's folder, not the cache directory the
  spec names; the folder is writable by then, and the path needs no `VCMP_HOME`.
- The old program deletes `vcmp.old` as soon as the new one confirms, so a version
  that crashes later has no automatic way back (the spec's open question).
- Confirmation fires at the first event-loop tick, not when the window is known to
  be painted; a crash between the two goes unnoticed.
- A second VCMP running from the same folder isn't detected; it keeps running the
  old program.
- Copies released before this change (26.11.0 and older) only link; users update by
  hand once.
- Error details in the failed dialog are the system's English messages, not
  translatable strings.

## Recommended follow-ups

- Make the key and set the secrets, then publish one signed release and run the
  manual steps in docs/manual-testing.md on Windows and Linux.
- Keep `vcmp.old` until the next normal start, so a version that crashes later can
  be rolled back too.
- A single-instance guard, so the swap can't leave a second window on the old
  program.
- Code signing on Windows, once there is a certificate, so updated programs don't
  meet SmartScreen.

## Changelog

- Install a newer VCMP from inside the app on Windows and Linux: the update dialog
  downloads it, checks its signature, and restarts into it
- Put the old version back automatically when a new one fails to start
- Add "Include pre-releases" in Settings, Updates, to be offered test versions
- The update note now opens the update dialog instead of the release page
