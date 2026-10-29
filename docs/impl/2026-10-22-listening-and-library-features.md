# Listening and library features

Spec: docs/specs/2026-10-22-listening-and-library-features.md
Date: 2026-10-22
Status: partially implemented
Area: vcmp-playback (engine), vcmp-app (player bar, player service, icons, screenshots)

## What was done

The sleep timer, built first because it was the one feature that did not wait for
PR #42 (merged since). It is on `feature/sleep-timer`, brought up to date with
`main` at 26.10.2.

- **Engine** (`crates/vcmp-playback/src/engine.rs`, `player.rs`):
  - `Command::StopAfterCurrent(bool)` makes `prepare_next` see no next track, and
    cuts a next track already in the ring with the existing `Control::Cut`.
  - At the boundary the queue advances and the engine stops with the next track
    waiting at its start. `TrackChanged` names that track with no duration.
  - The flag is cleared once it has stopped, by Stop, and at the end of the queue.
    Each change is reported with `Event::StopAfterCurrent`.
- **App** (`crates/vcmp-app/src/player_service.rs`):
  - `PlayerState.set-sleep-timer` handles the choices: minutes, -1 for "End of this
    track", 0 for off.
  - A repeated one-minute Slint timer counts down. When it runs out it asks the
    engine to stop after the current track, or simply ends if nothing plays.
  - Playback stopping ends a running timer.
- **UI** (`ui/player-bar.slint`, `ui/player-state.slint`, `ui/icons/sleep.svg`):
  - A moon button next to the queue button. Its menu offers "End of this track" and
    15, 30, 45, 60 and 90 minutes, plus "Turn off sleep timer" while a timer is on.
  - While on, the moon is in the accent; while counting down, the minutes left sit
    in its hollow.
  - The track section's minimum went from 160 to 136 px, so the bar still fits
    720 px.
- **Tests:**
  - Five engine tests: the next track cut and none of it heard, the request made
    while the current track is still being decoded, taking the stop back keeps
    playback gapless, Repeat one stops, and Stop and the end of the queue clear the
    flag.
  - Three app tests: the countdown, end of track and off, and paused or stopped.
- **Screenshot shots:** `sleep-timer-light` and `sleep-timer-dark`.
- **Docs:** `docs/playback.md` (Queue, Data path), `docs/ui.md` (Playback, the bar's
  width), `docs/manual-testing.md` and `README.md`.
- **Checks, on Windows:**
  - `cargo fmt --all --check` is clean.
  - `cargo test --workspace`: 427 passed, 0 failed (six full runs before the last
    engine test was added, one after). One earlier run printed failure markers that
    weren't kept and didn't come back.
  - `cargo clippy --workspace --all-targets -- -D warnings` fails on two things
    already on `main`, and nothing in this change:
    - `clippy::type_complexity` in `vcmp-integrations/src/reduce_motion.rs`, which
      clippy 1.99 now flags (it fails the same with this change stashed);
    - `App::failures` in `tests/tag_editor.rs`, used only under `#[cfg(unix)]` and so
      dead code on Windows.
  - With `-A clippy::type_complexity`, the tag editor test is the only error.
  - After `main` (26.10.2) was merged in, all run again:
    - `cargo fmt --all --check` is clean.
    - `cargo test --workspace`: 510 passed, 0 failed, including the eight sleep timer
      tests and `smallest_window_keeps_everything_inside`.
    - Clippy shows the same two errors, both already on `main`.
    - The sleep timer screenshots look right in both themes.

## Follow-up changes

1. Bring the branch up to date with `main` (26.10.2: MCP, labels, player marks,
   zoom). Main moved under the uncommitted work, so the work was put back on top
   and eight conflicts were resolved by keeping both sides:
   - the new icons next to the moon;
   - main's `TrackMark` next to `SleepButton` in the player bar;
   - `ScrobbleMark` beside `SleepTimer` in imports;
   - the agents and sleep shots in the screenshot example;
   - the bar's marks and the sleep timer in `docs/ui.md`.

## How to test

**Automated coverage.** `cargo test -p vcmp-playback --lib stop_after` and
`cargo test -p vcmp-app --test player -- sleep_timer end_of_this_track`. The repository
has no end-to-end suite that drives the real window, so the steps below are manual.

**Setup.** A library with at least one album of two or more tracks, best a gapless
one (a live or concept album). `cargo run -p vcmp-app`.

**Steps.**
1. Play the album from its first track. The moon button sits between the queue
   button and the speaker in the player bar, drawn like the other icons.
2. Click the moon. A menu opens fully inside the window and lists "End of this
   track", 15, 30, 45, 60 and 90 minutes.
3. Choose "End of this track". The moon turns to the accent colour. Its spoken
   label (screen reader, or Tab to it) is "Sleep timer, stops after this track".
4. Let the track play to its end. Nothing of the second track is heard. The bar
   shows the second track at 0:00 with the play button, and the moon is back to
   normal.
5. Press Play. The second track starts from its beginning.
6. Open the moon's menu and choose 15 minutes. "15" appears in the accent at the
   moon's top right, and the label reads "Sleep timer, 15 minutes left".
7. Wait a minute: it shows 14. After 15 minutes the number disappears, the moon
   stays in the accent, and the track playing then plays to its end and stops as in
   step 4.

**Also worth trying.**
- "Turn off sleep timer" while counting: the number goes away and playback carries
  on past the track end.
- Choose 15 minutes, then "End of this track": the countdown is replaced, and it
  stops at the end of the current track.
- Pause while a timer runs and let it run out: nothing happens when you resume
  later, and the moon is off.
- Next during "End of this track": it stops after the track skipped to instead.
- Repeat one with "End of this track": it stops instead of repeating.
- The last track of the queue with "End of this track": the queue ends as usual,
  with nothing waiting in the bar.
- The menu by keyboard: Tab to the moon, Enter opens it.
- A 720 px wide window: the bar's buttons all stay inside.

## What is missing

- Play history, favourites, the Home page, more formats, the library check, M3U and
  the first-run wizard. PR #42, which they waited for, is merged; each comes as its
  own PR.

## Known problems

- **Not yet tested by hand:** merged into `main` on the automated checks alone. No
  one has tried it in the real app or heard it on a real device yet. The steps in
  "How to test" above and the two sleep timer items in `docs/manual-testing.md`
  are still open.
- **Menu placement:** the menu opens at the button's top-left corner and relies on
  Slint keeping popups inside the window. That was not seen in a real window (the
  screenshot example can't open menus).
- **Counting:** the countdown is in whole minutes on the UI loop's timer, so it can
  drift by a few hundred milliseconds. A system that sleeps mid-countdown resumes
  counting from where the timer stood.
- **Timer in a paused state:** a timer that runs out while paused ends without
  stopping anything. A track resumed later plays on, which the spec did not decide
  either way.
- **Waiting track has no duration:** the engine announces the waiting track
  without its duration. The bar takes it from the library, so a file outside the
  library would show 0:00 until it plays.
- **Clippy on Windows:** the two pre-existing errors above.

## Recommended follow-ups

- Fix the two clippy errors on `main`: alias the type in `reduce_motion.rs`, and
  gate `App::failures` with `#[cfg(unix)]`. They block the documented pre-merge
  check on Windows.
- Start play history, the next phase of the spec. `main` is at schema version 12
  (labels), so its migration is 13.

## Changelog

- Add a sleep timer in the player bar that stops the music at the end of the current
  track, or after 15, 30, 45, 60 or 90 minutes once the track playing then ends
