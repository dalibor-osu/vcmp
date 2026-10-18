# Manual testing

What the automated tests can't check: how VCMP looks, sounds and behaves on a
real desktop, with real devices, real online services and the other two operating
systems. Work through it before a release and after changes to the areas named.
Tick items off in a copy, not here.

Items marked **(never seen working)** are built but have only been checked by
reading the code or with a simulated system, so they are the most likely to fail.

## Before you start

- Anything that writes tags works on copies. Copy a few albums into a folder of
  their own and use a separate `VCMP_HOME`, so the test library and its settings
  stay apart from your own:

  ```bash
  mkdir -p target/manual-test/music
  ```

  ```bash
  VCMP_HOME=target/manual-test cargo run -p vcmp-app
  ```

  Then add `target/manual-test/music` in Settings. Never use Edit tags on the real
  collection. Everything else (scanning, playback, covers and photos found online,
  playlists) only reads the music files.
- Last.fm and "Find photo online" exist only in builds made with the keys (README,
  "Integration keys"): set `VCMP_LASTFM_API_KEY`, `VCMP_LASTFM_SHARED_SECRET` and
  `VCMP_FANARTTV_API_KEY` in the shell that runs `cargo run`.
- Note the platform, the desktop (GNOME, KDE, ...) and the commit you tested.

## Every platform

### Library

- [ ] Add a music folder: albums appear while the scan runs, with progress in
      Settings, and the grid scrolls smoothly once it's done.
- [ ] Albums, Artists (album artists and all artists), Songs and Folders show the
      same music; the filters narrow each page.
- [ ] Ctrl+F (Cmd+F on macOS) searches artists, albums, playlists and tracks.
- [ ] With "Update automatically" on, copy an album into the folder, then delete
      it: it appears and disappears without a rescan.
- [ ] Move an album to another subfolder: a playlist that holds its tracks still
      plays them.
- [ ] Replace an album's `cover.jpg` with another picture from a file manager or
      image editor: the new cover shows.
- [ ] Remove a music folder: its albums go, and playlist entries from it stay,
      dimmed.

### Playback

- [ ] An album plays through without a gap between tracks (a live or concept album
      makes a gap easy to hear). MP3, FLAC, WAV and Ogg Vorbis all play.
- [ ] Seek with the slider and with the keyboard; pause, resume, previous and next;
      volume and mute; repeat off, all and one.
- [ ] Shuffle, "Fewer repeats" and "Fully random": no track comes back before the
      rest have played with Fewer repeats.
- [ ] Space plays and pauses from anywhere except a text field, also after
      clicking around and after closing the now playing view.
- [ ] Queue panel: it slides in (at once with Reduce motion on); Play next and Add
      to queue land in the right places; drag an Up next track to another place;
      Clear; Delete and Alt+Up/Down on the focused row.
- [ ] Quit while paused mid-track and start again: the queue comes back, paused at
      the same position.
- [ ] Settings, Output device: "System default" is first, then the outputs. Pick
      another output while a track plays: the sound moves.
- [ ] Unplug the chosen output (USB or Bluetooth) while playing: sound continues on
      the default, and Settings names the device playing instead. Plug it back and
      choose it again.

### Selecting tracks

- [ ] On an album page, Ctrl+click (Cmd+click) several tracks, then Shift+click:
      the selection shows, and a plain click plays and clears it.
- [ ] Keyboard: Shift+Up/Down, Shift+Home/End, Ctrl+A (Cmd+A), Ctrl+Space toggles
      one track, Escape clears, a second Escape goes back.
- [ ] The menu of a selected track says how many are selected, and Play next, Add
      to queue, Add to playlist and Edit tags take all of them.
- [ ] The same in the Songs table, and Remove from playlist on a playlist page.

### Tag editor (copies only)

- [ ] Edit one track, a whole album and a selection of tracks; the subtitle says
      which. Only the fields you changed are written; other players and a rescan
      show the new tags.
- [ ] A field that differs between tracks shows "Several values" and keeps the
      values unless you type one.
- [ ] Change and remove the embedded cover.
- [ ] Edit the track that is playing.
- [ ] Make a file read-only, then save: the editor names the file and the reason,
      and Try again works once it's writable.

### Playlists

- [ ] Make a playlist from New playlist and from a track's menu; rename it, delete
      it.
- [ ] Reorder by dragging and with Alt+Up/Down.
- [ ] The picture is made from the first four album covers; choose your own and
      go back.
- [ ] Delete one of the files: its entry is dimmed and skipped while playing.

### Covers and artist photos

- [ ] An album without a cover: Find cover online shows choices, the one picked
      stays after a restart, and Find another cover and Remove cover work.
- [ ] Artist page: Choose photo… with your own picture and with a file that isn't
      a picture (it says so); Remove photo.
- [ ] Find photo online (build with the fanart.tv key) for a well-known artist,
      an obscure one, and one with a name that several artists share.
      **(never seen working with a live key)**

### Window, look and keyboard

- [ ] Light, dark and system theme, with the system switched while VCMP runs.
- [ ] Reduce motion on System: switch the operating system's setting while VCMP
      runs and check that page fades stop and start (Windows: "Animation effects";
      macOS: "Reduce motion"; GNOME: "Animations"; KDE: animation speed
      "Instant"). **(never seen working on Windows, macOS or KDE)**
- [ ] Tab reaches every control in order and focus rings show only after a key
      press; a click hides them.
- [ ] Make the window as short as it goes: the sidebar scrolls and the focused
      entry stays in view.
- [ ] Long texts wrap without losing their last lines: Settings descriptions, the
      tag editor's error messages, the empty library message in a narrow window.
- [ ] Now playing view: the large cover, the "…" menu (Add to playlist), closing
      with Escape.

### Online

- [ ] Discord running, Show on Discord on: the status shows the track, progress
      and album cover; pausing clears it; quitting Discord and starting it again
      brings it back.
- [ ] Last.fm (build with the keys): Connect…, allow VCMP in the browser, play a
      track past half its length: it shows on last.fm as now playing, then as a
      scrobble. **(never seen working with a real account)**
- [ ] Last.fm offline: play a few tracks without network, reconnect: the plays
      arrive. Revoke VCMP's access on last.fm: the row asks to connect again.
- [ ] Update check: once the repository is public and has a release newer than the
      build, the sidebar and About show "VCMP x.y.z is available" and open its
      page; turning the setting off hides it. While the repository is private the
      check gets 404 and shows nothing.

## Linux

- [ ] Media keys and the desktop's media widget (GNOME's, KDE's) show the track
      and cover and control play, pause, next, previous, seek, volume, shuffle and
      repeat; Raise brings the window back; Quit closes VCMP.
- [ ] Output device list: no rate converters, mixers or "Discard all samples".
      Choosing a sound card that PipeWire is using shows that it's busy; PipeWire
      Sound Server and System default play.
- [ ] AppImage and .deb from a release: install, start from the menu, the icon
      shows in the menu and in the media widget, uninstall.

## Windows (never compiled locally)

- [ ] It builds and runs: `cargo run -p vcmp-app`, or start
      `.github/workflows/ci.yml` by hand.
- [ ] Media keys and the media flyout show the track and control it.
      **(never seen working)**
- [ ] Edit the tags of the playing track: the editor says it waits, the write
      happens when playback stops or moves to the next track, and pause alone
      doesn't release it. **(never seen working)**
- [ ] Last.fm keeps its key in Credential Manager across restarts.
- [ ] Output device list shows the WASAPI device names.
- [ ] MSI and the NSIS setup exe install, start, and uninstall cleanly.

## macOS (never compiled locally)

- [ ] It builds and runs.
- [ ] Now Playing in Control Center and the media keys show the track and control
      it; the cover shows. **(never seen working)**
- [ ] Cmd+click, Cmd+A and Cmd+F work where Ctrl does elsewhere.
- [ ] Last.fm keeps its key in the Keychain across restarts.
- [ ] The dmg opens, the app starts (unsigned: right-click, Open), and the icon
      shows.
