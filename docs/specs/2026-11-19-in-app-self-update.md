# In-app self-update

Status: agreed
Date: 2026-11-19
Area: vcmp-integrations (update check, download, verification, swap), vcmp-app
(update dialog, Settings, start-up handshake), release workflow and
`packaging/release-local.sh` (signing); docs (development.md, ui.md, README.md,
manual-testing.md); Windows and Linux archives only

## Context of the change

VCMP is released as unsigned archives on GitHub Releases: a zip of `vcmp.exe` for
Windows and a tar.gz of `vcmp` for Linux, each with the licence texts, plus a
`SHA256SUMS.txt` (`.github/workflows/release.yml`, and `packaging/release-local.sh`
while Actions can't run; docs/development.md, "Releases"). There is no installer:
the user unpacks the archive anywhere and runs VCMP from there (README.md,
"Download"). Recent releases carry no macOS build.

What exists for updates is a check, not an updater:

- `crates/vcmp-integrations/src/updates.rs` asks GitHub's release list at start,
  takes the highest published `vYY.MM.PATCH` tag, skips drafts and pre-releases,
  and reports it when it is newer than the running version.
- `crates/vcmp-app/src/updates.rs` and `ui/updates.slint` show "VCMP x is available"
  in the sidebar and on the About page; it opens the release page in the browser.
  A "Check for updates" switch (Settings and the setup wizard, setting
  `integrations.update_check`) turns it off; the first start holds the check until
  the wizard is done.
- The saved queue (`src/saved_queue.rs`) brings the queue back after any restart,
  paused at the saved position.

The v1 specification (docs/specs/2026-10-17-vcmp-music-player.md) chose "check +
notify" and named a self-updater as substantial work for later. Issue 109 asks for
that later step: install VCMP once, and let it update itself from GitHub Releases.
This specification supersedes the v1 decision for Windows and Linux.

## User request

Add support for self-updating, so the user can install the app once and it is able
to automatically update itself from the GitHub releases (issue 109). Brainstorm the
feature into a specification, in a new worktree and feature branch.

## Decisions

### Self-update for VCMP (issue 109): once a newer GitHub release is found, what should VCMP do?

| Option | What it means |
|---|---|
| Ask, then install (Recommended) | Sidebar note becomes 'Update to x'; one click downloads, replaces the program and restarts. User stays in control of when playback is interrupted; one extra click. |
| Download silently, apply on exit | Downloads in the background and swaps the program when VCMP closes; next start is the new version. Zero clicks, but the user never sees it happen and a bad release reaches everyone. |
| Both, chosen by a setting | A setting picks between asking and automatic. Covers both kinds of users; two code paths and one more setting to test and explain. |

**Answer: Ask, then install**

### Which platforms must self-update cover in this change?

| Option | What it means |
|---|---|
| Windows and Linux (Recommended) | The two archives actually published today. macOS keeps the 'is available' link until its builds are released again; smallest scope that serves real users. |
| Windows, Linux and macOS | Also replaces VCMP.app in place. Complete, but macOS builds aren't released now and an unsigned replaced .app may trip Gatekeeper/quarantine, so it can't be tested end to end yet. |
| Windows only | Linux users usually expect a package manager to update software; leaves Linux on the link. Smallest, but the tar.gz users get nothing. |

**Answer: Windows and Linux**

### Self-update downloads and runs a new program from GitHub. How should VCMP check that the downloaded archive is genuine before it replaces itself?

| Option | What it means |
|---|---|
| Signed checksums (Recommended) | The release signs SHA256SUMS.txt with an ed25519 (minisign) key; the public key is compiled into VCMP, which checks the signature and the hash. Stops a tampered release or hijacked account; one private key to guard and use in both release paths. |
| SHA256SUMS.txt only | Compare the archive with the release's own checksum file. No key to manage; catches corrupt downloads but not a tampered release, since the attacker replaces both files. |
| Rely on HTTPS | Trust GitHub's TLS download as is. Nothing to build; a compromised release or account installs anything on every user's machine. |

**Answer: Signed checksums**

### Today VCMP is a zip/tar.gz the user unpacks anywhere. Where does the self-update put the new version?

| Option | What it means |
|---|---|
| Replace in place (Recommended) | Swap the running vcmp(.exe) wherever it was unpacked, keeping the old one beside it until the new one starts. No installer needed; where the folder isn't writable (Program Files, /usr, a package) VCMP falls back to today's link. |
| Own per-user install folder | VCMP offers once to move itself to %LOCALAPPDATA%\Programs\VCMP or ~/.local, with a Start menu entry or .desktop file, and updates there. A real 'install once'; adds an install step, shortcuts and an uninstall story. |
| Switch to installers | Releases ship per-user NSIS (Windows) and AppImage (Linux); the update downloads and runs the installer or swaps the AppImage. Proper install UX; changes the release artefacts and both release paths, much bigger scope. |

**Answer: Replace in place**

### If the new version fails to start after the swap (a broken build, a missing library on that Linux), what happens?

| Option | What it means |
|---|---|
| Restore automatically (Recommended) | The old VCMP launches the new one and waits for it to report its window is up; if it doesn't within a timeout, the old program is put back and restarted with an error note. Safe for users; a small handshake between the two versions to build and test. |
| Keep the old one, manual | The previous program stays beside it as vcmp.old(.exe) until the next update; the docs say how to rename it back. Simple; a user with a broken update has to find and rename a file. |
| No rollback | The old program is deleted after the swap. Simplest; a bad release leaves users with nothing that runs. |

**Answer: Restore automatically**

### Music is playing when the user clicks to install. What happens to playback across the restart?

| Option | What it means |
|---|---|
| Resume where it was (Recommended) | The new version restores the saved queue at the saved position and keeps playing if it was playing. Feels like a short pause; needs a 'was playing' hint passed to the new process. |
| Restore paused | Same as any restart today: the queue comes back paused at the saved position. Nothing new to build; the user presses play again. |
| Wait until playback stops | Clicking during playback schedules the install for when the music is stopped or VCMP is closed. Never interrupts; the update can sit pending for a long time. |

**Answer: Restore paused**

### Where does the user start the update and see its progress? (Today a 'VCMP x is available' link sits in the sidebar and on the About page.)

| Option | What it means |
|---|---|
| Dialog from the note (Recommended) | The sidebar/About note opens a dialog: version, link to the release notes, 'Install and restart', then download progress and errors in place. Clear, cancellable, keyboard-friendly; one new dialog to design in every theme. |
| One click in the sidebar | The note itself becomes 'Update to x', downloads with inline progress and restarts. Fastest; easy to click by accident, no room for notes or errors. |
| About page only | The sidebar note stays a hint; the install button and progress live on the About page. Unobtrusive; the action is two steps away and easy to miss. |

**Answer: Dialog from the note**

### Should self-update offer pre-releases (GitHub releases with the pre-release box ticked) to users who opt in?

| Option | What it means |
|---|---|
| No, releases only (Recommended) | Same rule as the update check today: pre-releases are skipped. Testers download them by hand; no new setting, and a pre-release can't reach ordinary users. |
| Opt-in switch in Settings | 'Include pre-releases' makes the check and the update consider them. Easy testing of releases; one more setting, and testers can't step back to the release line without a manual download. |

**Answer: Opt-in switch in Settings**

### How is the download, check and swap built?

| Option | What it means |
|---|---|
| Own module + self-replace (Recommended) | Extend vcmp-integrations' update check with ureq download, zip/tar extraction, minisign verify, and the small self-replace crate for the Windows-safe swap. Fits the existing check, own rollback handshake; more code of our own to test. |
| self_update crate | Use the self_update crate's GitHub backend for finding, downloading and replacing. Less code; its own release lookup duplicates ours, its signing is zipsign not minisign, and rollback still has to be added around it. |

**Answer: Own module + self-replace**

## High-level plan

**1. Signed releases.** A minisign key pair is created once. Both release paths (the
Release workflow and `packaging/release-local.sh`) sign `SHA256SUMS.txt` and attach
`SHA256SUMS.txt.minisig` to the release. The public key goes into the source; the
private key stays a repository secret and on the owner's machine. docs/development.md
gets the key handling and the extra release step. Nothing else depends on the app,
so this comes first: the first release that self-updates must already be signed.

**2. Finding the update, pre-releases included on request.** The existing check
learns to also return the release's archive for this platform and its signature
files, and to consider pre-releases when the new "Include pre-releases" switch in
Settings is on. Depends on nothing but the current check.

**3. Download, verify, swap and rollback.** In vcmp-integrations, with no Slint: the
download with progress and cancel, the signature and hash check, extraction to the
program's folder, the swap, launching the new version and the start-up handshake
that confirms it or restores the old one. Depends on 1 (the signature format) and 2.

**4. The update dialog.** The sidebar and About notes open a dialog with the version,
a link to the release notes and "Install and restart"; it shows progress, errors and
the reason when VCMP can only link to the release. Before the restart the queue is
saved as on any exit and comes back paused. Depends on 3.

**5. Documentation and manual tests.** README (updating), docs/ui.md (dialog,
switch), docs/development.md (signing), docs/manual-testing.md (a real update and a
forced rollback on Windows and Linux). Finishes with 4.

## Architecture decisions

**The updater lives in vcmp-integrations, the dialog in vcmp-app.** It extends
`updates.rs` there, beside the check it builds on, with no Slint and no knowledge
of the window; the app hands it callbacks for progress and the result, as the check
does today. This keeps the core-crate rule from CLAUDE.md and lets the download,
verification and swap be tested headless against local files.

**Trust anchor: a compiled-in minisign public key.** VCMP installs only an archive
whose SHA-256 is listed in a `SHA256SUMS.txt` whose minisign signature verifies with
the key compiled into the running build. HTTPS and GitHub are transport, not trust.
Signing the checksum file instead of each archive keeps one signature per release
and reuses the file both release paths already write. A release without a valid
signature, or without an archive for this platform, is never installed: the dialog
explains it and offers the release page, as today. Minisign beat the self_update
crate's zipsign because it is a small, documented format with a tiny verify-only
crate and a command-line tool for the release scripts.

**In-place swap of the program only.** The new program is extracted next to the
running one, the running one is renamed aside (`vcmp.old`, `vcmp.old.exe`), and the
new one takes its name. On Windows a running exe can be renamed but not overwritten
(the self-replace crate handles this); on Linux the rename replaces the running file
safely and the executable bit is kept. The licence texts next to the program are
refreshed with it. No other file is touched, so settings, the library database and
caches are never part of an update.

**When VCMP can only link.** VCMP offers to install only when: it runs on Windows or
Linux, it is a release build (debug builds keep the link, so `cargo run` never
replaces `target/`), and the program's folder is writable. Otherwise, including
macOS, a read-only or package-managed location and a missing signature, the note
and dialog link to the release page as today, with the reason shown in the dialog.

**Hand-over and rollback handshake.** After the swap the old VCMP saves the queue,
stops playback and closes its window and everything that holds shared resources
(library database, MCP port, media controls, Discord), then starts the new program
with an argument naming the update, and waits without a window. The new VCMP, when
its window is shown, confirms through a marker in the cache directory; the old one
then exits and the new one removes `vcmp.old`. If no confirmation arrives within a
timeout, or the new process exits first, the old VCMP ends it, puts its own program
back, and starts again showing that the update failed. The timeout and the marker
format are implementation details; the invariant a test enforces is that after any
outcome exactly one program sits under the original name and it starts.

**Restart restores paused.** The new version starts like any other start: the saved
queue comes back paused at the saved position. No playing state crosses the
hand-over.

**Settings.** "Check for updates" keeps its meaning and stays the only switch that
lets VCMP ask GitHub; with it off there is no check and no dialog. "Include
pre-releases" is a new switch in the Updates section of Settings only (not in the
setup wizard), saved in the library settings next to the check's, off by default.
It changes which releases the check considers; nothing is downloaded without the
user's click in the dialog.

**Network policy.** Unchanged in spirit: the check stays on the existing switch, and
the download happens only after the user clicks "Install and restart". It reaches
GitHub's release asset host besides api.github.com.

## Weaknesses and risks

| Risk | Likelihood | Cost | What reduces it |
|---|---|---|---|
| The private signing key is lost | Low | Releases after it can't be installed by self-update; users must download once by hand | Back it up offline; document where it lives; a key change ships a build that carries the new key, installed by hand once |
| The private key leaks | Low | Anyone can push an update to every copy with self-update | Keep it out of the repository and logs, password-protect it, rotate as above; the release still also needs GitHub access |
| Antivirus or SmartScreen flags an unsigned exe that replaces itself | Medium on Windows | The swap fails or the new exe is quarantined | The rollback restores the old program; code signing stays a later change; manual test on a stock Windows Defender |
| The new version fails only after the window is up (crash a minute later) | Low | The handshake already confirmed it; no automatic rollback | `vcmp.old` is gone by then; the user downloads by hand; consider keeping `vcmp.old` until the next start in the implementation |
| A second VCMP instance is running (there is no single-instance guard) | Medium | The other instance keeps the old program open; the swap still works by rename, but two versions share one library | The dialog warns when it detects a second instance if that is cheap; otherwise documented |
| Pre-release testers can't step back to the release line | Certain for testers | They stay on the pre-release until a newer release | Documented next to the switch; a manual download fixes it |
| Copies released before this change (26.11.0 and older) only link | Certain | Users update by hand once to get self-update | Release notes of the first self-updating version say so |
| Linux build needs a newer glibc than the user has | Low | The new program doesn't start | The rollback handshake restores the old one and reports it |
| macOS stays on the link | Certain | No self-update for Mac users | Out of scope; revisit when macOS builds are released and signed |

Open questions, left for the implementation: the handshake timeout; whether
`vcmp.old` is kept until the next start; whether a second running instance can be
detected cheaply. None changes the shape of this design.

## Out of scope

- macOS self-update; it keeps the "is available" link.
- Installers (MSI, NSIS, dmg, AppImage, .deb) and package-manager channels; the
  cargo-packager configuration stays as it is.
- Code signing of the programs (Authenticode, notarization); only the checksums are
  signed.
- Silent or background installation, and an "install on exit" mode.
- Resuming playback after the restart.
- Downgrading, choosing a specific version, and stepping back from pre-releases.
- Updating the VCMP streaming server (docs/specs/2026-11-11-vcmp-streaming-server.md)
  or anything other than the desktop program.
- A single-instance guard for VCMP.
