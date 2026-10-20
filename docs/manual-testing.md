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
- Last.fm and "Find photo online" exist only in builds made with the keys
  (docs/development.md, "Integration keys"): set `VCMP_LASTFM_API_KEY`,
  `VCMP_LASTFM_SHARED_SECRET` and `VCMP_FANARTTV_API_KEY` in the shell that runs
  `cargo run`.
- Note the platform, the desktop (GNOME, KDE, ...) and the commit you tested.

## Every platform

### Library

- [ ] Add a music folder: albums appear while the scan runs, with progress in
      Settings, and the grid scrolls smoothly once it's done.
- [ ] Albums, Artists (album artists and all artists), Songs and Folders show the
      same music; the filters narrow each page.
- [ ] Tracks without an album tag show in an "Unknown" album of their artist, with
      a question mark for a cover, after the artist's other albums. Tracks with no
      artist either are in an "Unknown" album by "Unknown artist", last in the
      grid. Give such a track an album in the tag editor: it moves there.
- [ ] Ctrl+F (Cmd+F on macOS) on Albums, Artists or Songs puts the focus in the
      page's filter; on any other page it opens Search.
- [ ] The mouse's back and forward buttons (and Alt+Left/Right) move through the
      sections and album, artist and playlist pages you opened, like a browser.
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
- [ ] Spin the mouse wheel fast (a free-spinning wheel, if you have one) on Albums,
      Songs and a long album: the page scrolls at once and glides to a stop, with
      no freeze. A touchpad still flicks.
- [ ] Seek with the slider and with the keyboard; pause, resume, previous and next;
      volume and mute; repeat off, all and one.
- [ ] Shuffle, "Fewer repeats" and "Fully random": no track comes back before the
      rest have played with Fewer repeats.
- [ ] Space plays and pauses from anywhere except a text field, also after
      clicking around and after closing the now playing view.
- [ ] Queue panel in a window narrower than 1000 px: it covers the whole page area
      but not the sidebar, Tab doesn't reach the page under it, and Escape closes
      it. Wider than that, it sits beside the page.
- [ ] Queue panel: it slides in and out (at once with Reduce motion on); Play next and Add
      to queue land in the right places; drag an Up next track to another place,
      also past the visible rows (the list scrolls while you hold it at the edge);
      Clear; Delete and Alt+Up/Down on the focused row.
- [ ] Quit while paused mid-track and start again: the queue comes back, paused at
      the same position.
- [ ] Settings, Output device: "System default" is first, then the outputs. Pick
      another output while a track plays: the sound moves.
- [ ] Unplug the chosen output (USB or Bluetooth) while playing: sound continues on
      the default, and Settings names the device playing instead. Plug it back: on
      Linux with a sound server, sound returns to it within half a second
      **(never seen working with a real device)**; elsewhere, choose it again.

### Selecting and playing tracks

- [ ] A click selects a track and plays nothing. Under the pointer the track's
      number turns into a play button, which plays from that track; so does a
      double-click.
- [ ] On an album page, Ctrl+click (Cmd+click) several tracks, then Shift+click:
      the selection shows, and a plain click leaves only that track selected.
- [ ] Keyboard: Shift+Up/Down, Shift+Home/End, Ctrl+A (Cmd+A), Ctrl+Space toggles
      one track, Escape clears, a second Escape goes back.
- [ ] The menu of a selected track says how many are selected, and Play next, Add
      to queue, Add to playlist and Edit tags take all of them.
- [ ] The same in the Songs table (its play button is at the start of each row),
      and Remove from playlist on a playlist page.

### Track list columns

- [ ] In a wide window, album, artist, playlist, search and folder track lists
      reach the page's right edge.
- [ ] Drag the edge between the Title heading and the one beside it: both columns
      follow the pointer, and neither gets narrower than about 64 px. A
      double-click on the edge shares the room evenly. The Songs table has two
      edges, between Title, Artist and Album.
- [ ] Tab to an edge and move it with Left and Right; a screen reader names it as
      a slider ("Width of the Title column").
- [ ] Restart VCMP: the widths you set are kept.

### Tag editor (copies only)

- [ ] Edit one track, a whole album and a selection of tracks; the subtitle says
      which. Only the fields you changed are written; other players and a rescan
      show the new tags.
- [ ] A field that differs between tracks shows "Several values" and keeps the
      values unless you type one.
- [ ] Type a year like "abc" and save: the Year label turns bold and red, a red
      sentence under the form says what's wrong, and nothing is written.
- [ ] Rename the album whose page is open (its title or album artist): the page
      shows it under its new name, not as "no longer in your library".
- [ ] Save an album or a selection: the editor first lists the number of files and
      each changed field, and writes only after "Write N files". A second Enter,
      Back and Escape return to the form with your values; one track still writes
      at once.
- [ ] Change and remove the embedded cover.
- [ ] Edit the track that is playing.
- [ ] Make a file read-only, then save: the editor names the file and the reason,
      and Try again works once it's writable.
- [ ] On each platform, run `cargo run --release -p vcmp-library --example
      edit_check -- <music dir> 40` against your collection. It edits copies only,
      every other tag included, and should end with "0 failed edits"; each failure
      it prints names the file and the tag.

### Playlists

- [ ] Make a playlist from New playlist and from a track's menu; rename it, delete
      it.
- [ ] Reorder by dragging and with Alt+Up/Down. In a playlist longer than the
      window, hold a dragged track near the bottom, then the top: the page scrolls
      smoothly, faster nearer the edge, and the track lands where the line shows.
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
- [ ] A photo picked there says "Photo from fanart.tv" under it, and the link
      opens fanart.tv; a photo from a file says nothing. About names fanart.tv,
      the Cover Art Archive and MusicBrainz, and each link opens its site.
- [ ] Settings → Artist photos: paste your fanart.tv personal key, Save, and
      search again; photos still come. Type a wrong key: the picker says fanart.tv
      didn't accept the personal key. Remove it: searches work with VCMP's key
      alone. The key is kept after a restart.

### Window, look and keyboard

- [ ] Light, dark and system theme, with the system switched while VCMP runs.
- [ ] Light theme: hover over track rows, Songs table rows and headings, folder
      rows and queue rows: the grey text turns dark and stays easy to read.
- [ ] With no music folder, Albums, Artists, Songs and Folders each say so and
      offer "Add music folder…".
- [ ] Reduce motion on System: switch the operating system's setting while VCMP
      runs and check that page fades stop and start (Windows: "Animation effects";
      macOS: "Reduce motion"; GNOME: "Animations"; KDE: animation speed
      "Instant"). **(never seen working on Windows, macOS or KDE)**
- [ ] Tab reaches every control in order and focus rings show only after a key
      press; a click hides them.
- [ ] The sidebar has three groups, set apart by a gap and a line: Search; Albums
      to Folders; Settings and About. The page marker sits on the section shown.
- [ ] With an album, artist or playlist open, go to another section and back by
      its sidebar entry: the section opens on its grid.
- [ ] Make the window as short as it goes: the sidebar scrolls and the focused
      entry stays in view.
- [ ] Long texts wrap without losing their last lines: Settings descriptions, the
      tag editor's error messages, the empty library message in a narrow window.
- [ ] Hovering an album, artist or playlist card puts a fill behind it. Opening an
      album, artist or playlist page, and going back, fades the new view in.
- [ ] The queue button's icon sits centered in its round hover fill, and the
      speaker icon is level with the icons beside it.
- [ ] In the Songs table, a hovered row stays highlighted, without blinking,
      while the pointer moves and when it rests.
- [ ] Now playing view (fades in and out): the large cover (up to 880 px on a full screen, with a
      larger title in a very wide window), the "…" menu (Add to playlist), closing
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

- [ ] With the VCMP window focused, the keyboard's play/pause, next and previous
      keys control it on any desktop, a bare window manager too.
- [ ] Media keys and the desktop's media widget (GNOME's, KDE's) show the track
      and cover and control play, pause, next, previous, seek, volume, shuffle and
      repeat; Raise brings the window back; Quit closes VCMP.
- [ ] Output device list: the outputs carry the names the desktop's sound settings
      use ("Scarlett Solo (3rd Gen.) Headphones / Line 1-2"). Choose one while
      another app plays through it: both are heard, nothing is busy.
- [ ] With System default chosen, switch the default output in the desktop's sound
      settings while a track plays: the sound moves within half a second. On
      PipeWire it moves at once, with a short skip up to half a second later.
      **(seen only with a separate test server)**
- [ ] Restart the sound server while a track plays
      (`systemctl --user restart pipewire-pulse`, or `pulseaudio -k`): playback
      carries on after a short gap. **(seen only with a separate test server, and
      never with PulseAudio itself)**
- [ ] Without a PulseAudio server, start VCMP with
      `PULSE_SERVER=unix:/nonexistent cargo run -p vcmp-app`. Settings lists ALSA's
      devices, without rate converters, mixers or "Discard all samples". Choosing a
      sound card that PipeWire is using shows that it's busy. PipeWire Sound Server
      and System default play.
- [ ] The `linux-x64.tar.gz` from a release unpacks and `vcmp` starts, on another
      distribution than the one it was built for too. On a system without
      PipeWire or PulseAudio (a plain ALSA install, such as a minimal Debian VM),
      sound plays through ALSA. **(never seen working)**

## Windows (never run on Windows itself)

- [ ] It builds and runs: `cargo run -p vcmp-app`, or start
      `.github/workflows/ci.yml` by hand.
- [ ] Media keys and the media flyout show the track and control it.
      **(never seen working)**
- [ ] Edit the tags of the playing track: the editor says it waits, the write
      happens when playback stops or moves to the next track, and pause alone
      doesn't release it. **(never seen working)**
- [ ] Last.fm keeps its key in Credential Manager across restarts.
- [ ] Output device list shows the WASAPI device names.
- [ ] The `windows-x64.zip` from a release unpacks and `vcmp.exe` starts without
      a console window, after SmartScreen's More info → Run anyway.
- [ ] The same for the zip that `packaging/release-local.sh` cross-builds with
      MinGW, and the rest of this list with it: it plays, and the media flyout,
      Credential Manager and the output devices work. **(seen only starting under
      Wine on Linux)**

## macOS (never compiled locally)

- [ ] It builds and runs.
- [ ] Now Playing in Control Center and the media keys show the track and control
      it; the cover shows. **(never seen working)**
- [ ] Cmd+click, Cmd+A and Cmd+F work where Ctrl does elsewhere.
- [ ] Last.fm keeps its key in the Keychain across restarts.
- [ ] The `macos-arm64.zip` (or `macos-x64.zip`) unpacks to `VCMP.app`, which
      starts after System Settings → Privacy & Security → Open Anyway, and shows
      its icon.
