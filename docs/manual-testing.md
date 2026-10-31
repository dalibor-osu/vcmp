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
- [ ] While a long scan of a hard disk runs, turn a switch in Settings (AI agents,
      Discord) and make a playlist. After a restart, both are still there.
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
- [ ] Turn the mouse wheel over the volume slider: up raises the volume, down lowers
      it, a notch at a time, also with a free-spinning wheel and a touchpad. Over
      the position slider the wheel doesn't seek. Restart: the volume set by wheel
      is kept.
- [ ] Play a FLAC or WAV file: a small waveform mark follows the artist in the
      player bar, and pointing at it says "Lossless (FLAC)". An MP3 or Ogg file has
      none. With Last.fm connected, a longer track shows the Last.fm mark, which
      turns to a tick once half of it (or 4 minutes) has played.
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
- [ ] Sleep timer: the moon button's menu opens fully inside the window (it sits
      at the bottom edge), by click and by Enter. "End of this track" during a
      gapless album: the track ends, nothing of the next one is heard, and the bar
      shows the next track stopped at 0:00; Play starts it from its start.
- [ ] Sleep timer, 15 minutes: the minutes left count down in the moon; when they
      run out mid-track, the track plays to its end and stops there. "Turn off
      sleep timer" before that: playback goes on.
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

- [ ] The editor opens as a dialog in the middle of the window, over the dimmed
      sidebar and pages, in both themes. Tab and Shift+Tab go round its controls
      and never reach the page behind; a click beside it does nothing. In the
      smallest window the form scrolls and Save and Cancel stay in view.
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
- [ ] **(never seen working)** Export… a playlist with that missing entry, a track
      in it twice and names outside ASCII (Dvořák, 日本) into the music folder, then
      into a folder elsewhere on the same drive. The save dialog starts with the
      playlist's name and `.m3u8`, and the page says where the file went and that
      one entry's file isn't there. Export again to the same name: the dialog asks
      before replacing it. On each of Windows, Linux and macOS:
  - Open both files in VLC: every entry that is there plays, in order, twice
    where it is twice, with the right titles. The file in the music folder has
    paths like `Album/01 Track.flac`.
  - Windows: open them in foobar2000 too. Export to a USB stick or another drive
    letter than the music: the paths are absolute (`D:\Music\…`).
  - Linux or macOS: export to a USB stick while the music is on the system disk:
    the paths are absolute.
  - Copy the music folder with the exported file in it to a phone, and open the
    playlist in its player: the relative paths find the files.

### Labels

- [ ] Settings → Labels → New label…: name it, pick a palette colour, Make label.
      Make another with Custom: drag in the shades and the hue strip, type a hex
      code; a wrong code says so. Both show in Settings, the sidebar and the Labels
      page with their dots, in the light and the dark theme.
- [ ] Right-click a track → Labels → the label: its chip shows after the title, in
      every list it is in, and in the Songs table's Labels column. Select three
      tracks, one of them labelled: the menu says "(on some)"; choosing it labels
      all three, choosing it again takes it off all.
- [ ] With the keyboard: focus a track list, press the Menu key (and Shift+F10),
      open Labels: the check marks match the current track.
- [ ] Right-click an album card on the Albums page and on an artist's page, and
      open an album page's three-dot menu → Labels: the label goes on every track
      of the album. New label… from there makes one and puts it on them.
- [ ] Songs → Labels: pick one label, then two: only songs with both stay; the
      chips beside the button take a label out. Albums → Labels: albums with at
      least one labelled track stay.
- [ ] Open a label from the sidebar: its tracks, Play, Shuffle; the queue panel
      names the label. Back, Escape and the mouse's back button return.
- [ ] Edit… renames and recolours; a taken name says so. Delete… asks, saying how
      many tracks lose it.

### Covers and artist photos

- [ ] An album page's More button ("…" beside Shuffle) opens a menu below it, by
      click and by Enter: Find cover online (or Find another cover and Remove cover),
      Add to playlist with every playlist, and Edit tags…. An artist page's holds
      Find photo online, Choose photo… and Remove photo. Nothing is left under the
      cover but the fanart.tv credit.
- [ ] On a full screen the cover of an album, artist and playlist page is 280 px,
      with the larger title beside it; narrowing the window to 720 px shrinks the
      cover first, and Play, Shuffle and More stay beside it.
- [ ] An album without a cover: Find cover online shows choices, the one picked
      stays after a restart, and Find another cover and Remove cover work.
- [ ] The cover and photo pickers are dialogs in the middle of the window that
      don't change size as results arrive; a click beside one closes it.
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

### Animated pictures

Use a few GIFs (one small, one large and long, say 800 px and 300 frames), an
animated WebP and an APNG. For album covers, edit copies only.

- [ ] Playlist: Choose picture… with a GIF. The page plays it at the GIF's speed;
      the Playlists grid, search and the sidebar show its first frame. Use album
      covers removes both files from `images/playlists/`.
- [ ] Artist: Choose photo… with an animated WebP. The artist page plays it; the
      Artists grid shows the first frame. Remove photo removes both files from
      `images/artists/`.
- [ ] Album: Edit tags… on a whole album, Choose image… with a GIF, Save. The album
      page and, while one of its tracks plays, the now playing view play it; the
      grid, the player bar and the queue show the first frame. Another player
      shows the first frame as the cover. Choosing another (still) cover ends the
      animation.
- [ ] An album whose files hold no cover and whose folder holds `cover.gif` plays
      it after a scan.
- [ ] Settings → Reduce motion On: every animated picture stops on its first frame
      at once; Off starts them again. The same with the OS setting and System.
- [ ] Leave an album page with an animated cover for the grid, another section or
      the now playing view: the animation stops (watch the CPU drop in a system
      monitor) and starts again on coming back.
- [ ] Memory and CPU: open the large, long GIF on an album page, then on a playlist
      page, and play a track of that album in the now playing view. Note VCMP's
      memory and CPU in a system monitor before and after; memory should go back
      down within a little while of leaving each page, and CPU should stay low with
      several animated pictures across pages, since only the one in sight plays.
- [ ] Release build: `cargo run --release -p vcmp-app --example grid_benchmark --
      5000 120` reports the same render times as before (the grid draws only
      stills).

### Window, look and keyboard

- [ ] Light, dark and system theme, with the system switched while VCMP runs.
- [ ] Settings → Zoom on a large monitor: + and − step through 80 to 200 %, the
      whole window grows and shrinks at once, the player bar stays at the bottom
      without resizing the window, and clicks still land where they point.
      Ctrl+=, Ctrl+- and Ctrl+0 do the same from any page (not while typing in a
      field is fine too). Restart: the zoom is kept. Move the window to a monitor
      with another scale: the zoom stays on top of it. **(seen on Linux under a
      tiling window manager, where the first build needed a resize to lay out;
      another monitor's scale not yet tried)**
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
- [ ] On a full screen, Settings and About sit in a column in the middle of the
      page, and the column narrows with the window down to 720 px.
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

### AI agents (copies of your music, or a VCMP_HOME library)

Use a library of copied files (`VCMP_HOME`, see CLAUDE.md): accepting writes tags.

- [ ] Claude Desktop: copy the configuration entry from Settings into
      `"mcpServers"` (Settings → Developer → Edit Config), restart Claude Desktop:
      vcmp's tools are listed and answer. Close VCMP and ask again: Claude says
      VCMP isn't open. **(never seen working with Claude Desktop; the relay was
      run with piped requests)**
- [ ] Settings → AI agents: turn it on. The address, token and the Claude Code
      command show; Copy puts each on the clipboard. Run the command in a
      terminal, then `claude mcp list` shows vcmp connected, in any folder. In the
      Claude desktop app's Code tab, vcmp's tools are still there after a
      session's second and third message. **(Claude Code connected and made a
      playlist; the tools lasted past the first message only while vcmp was in
      local scope, not yet seen in user scope)**
- [ ] Ask Claude Code to list your albums, to show every tag of a track, and to
      find tracks without a MusicBrainz release ID. The answers match the library.
- [ ] Ask it to fix something across an album (the album artist, the genre) and
      the titles of a few tracks. The sidebar says a change waits. Review shows
      Before and After in two columns: what every file gets once ("In all 12
      files"), then each file with its own changes under its title and file name.
      Decide later and Escape leave it waiting; Write writes it, the album and the
      player show the new tags, and Claude Code's get_change_set says accepted.
- [ ] Ask it to give an album a cover from the web (a Cover Art Archive address) and
      another from a file it downloaded. The review shows the picture once for the
      album; writing it shows the cover on the album page, in the grid and in the
      player bar. A web page address or a text file is refused with a reason.
- [ ] Ask for another change, then edit one of its files in the tag editor before
      accepting: that file is left as it was and named in the result.
- [ ] Settings → Changes by agents → Show changes… opens the list in a dialog;
      Escape, Close and a click beside it close it. Undo… there opens a review of
      the undo; writing it puts every value back, a file with two artists too, and
      closing the review brings the list back.
- [ ] Make a label "chill" and put it on a few tracks. Ask Claude Code which
      tracks are chill: it finds them by the label. Ask it to label a calm album
      "chill" and to make a new label for another: the review lists the labels put
      on (once for what every track gets, then per track) and the new label.
      Change puts them on; the chips show at once. Undo… in Show changes… takes
      them off and removes the new label.
- [ ] Ask Claude Code to set up labels (Favourite, Chill, Focus, Sleep) without
      labelling any track. The review says the change only makes labels, names
      them and has no table; Make 4 labels adds them to Settings → Labels with no
      tracks, and they can be put on tracks like any label. The list of changes
      shows it as "New labels"; Undo… there names the labels it deletes, and Undo
      deletes them except one put on a track meanwhile.
- [ ] Ask it to make a playlist and fill it: it appears at once. Ask it to delete
      one: the review warns that it can't be undone; Delete playlist deletes it.
      The playlist page's own Delete says the same.
- [ ] Playback (real device, Discord running, the OS media widget open): with
      "Let AI agents control playback" off, ask Claude Code to play something:
      it says playback control is off and where to switch it on. Turn it on.
      Ask it to "play Yor Zarad by Angine de Poitrine" (or any track): it plays at
      once, and the bar, the OS media widget, Discord and Last.fm's now playing
      show it; the queue panel says "Next from" the agent's name. Ask for "some
      chill music": it plays the label, and the queue panel names the label.
- [ ] Ask it to pause, resume, skip forward and back, seek to the middle, turn
      shuffle and repeat on, set the volume to 30 and to bring VCMP to the front:
      the bar's buttons, slider and the OS media widget follow each time, and the
      settings come back after a restart. Ask what plays: it names the track, the
      position and what comes next as the queue panel shows it. Ask it to skip
      and say what plays in the same breath (it reads `get_now_playing` right
      after `next`): it names the new track, not the one before.
- [ ] Ask it to play one track with repeat on all, then what comes next: it lists
      that track once and says the queue repeats, not the same track a hundred
      times. Ask how many tracks have no MusicBrainzReleaseId: it answers from one
      `find_tracks_by_tag` call (its `total`).
- [ ] Ask it to play next a track, add two to the end of Up next, then move and
      remove one: the queue panel follows.
- [ ] Unplug (or rename) the folder of one track of a playlist, then ask it to play
      the playlist: it plays the rest and tells you which track was left out and
      why. Ask for only missing tracks: it says none can be played.
- [ ] Ask it to list the output devices and switch to another (headphones, a USB
      DAC): the sound moves, Settings shows the new device, and it is still chosen
      after a restart. Ask for a device that isn't there: it says so.
- [ ] Ask it to set the sleep timer to 15 minutes, then to end of this track: the
      bar's timer shows each, and playback stops after the track.
- [ ] Close VCMP with a change waiting, start it again: the change shows as not
      reviewed before VCMP closed.
- [ ] Start a second program on the port (`python3 -m http.server 8267`), turn the
      switch off and on: Settings says the port is in use; another port in Port
      works. New token: the old one gets "401" (Claude Code fails to connect).
- [ ] Delete backups… asks first, then empties the list of finished changes.

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
- [ ] An agent's change that includes the playing track: the review names the file
      and Write stays disabled until playback moves on. **(never seen working)**
- [ ] The relay from a release build, which has no console window: Claude Desktop
      with `vcmp.exe` and `mcp` lists vcmp's tools. **(never seen working)**
- [ ] Last.fm keeps its key in Credential Manager across restarts.
- [ ] Output device list shows the WASAPI device names.
- [ ] The `windows-x64.zip` from a release unpacks and `vcmp.exe` starts without
      a console window, after SmartScreen's More info → Run anyway.
- [ ] No system title bar, on Windows 11 and 10: VCMP's minimise, maximise and
      close buttons sit in the top right corner, in both themes, and their glyphs
      dim while another window is active. Each one works, and hovering it fills
      it. Maximise turns into Restore while the window is maximised. Turn on
      Settings → Personalisation → Colours → "Show accent colour on title bars and
      window borders": nothing pink appears (Windows 11 also keeps a faint grey
      border). **(never seen working)**
- [ ] Hovering the maximise button on Windows 11 shows Snap layouts, and picking
      one snaps VCMP there. **(never seen working)**
- [ ] Drag the window by the strip along its top (over the page, above the title)
      and by the sidebar's top (the "VCMP" wordmark): it moves, snaps at the
      screen's edges, and a double-click maximises and restores it. A right-click
      there opens the window menu, and so does Alt+Space. With the now playing
      view or a dialog open, only the strip drags. **(never seen working)**
- [ ] Resize the window at all four edges and corners, the top included.
      Maximised, nothing is cut off at the top or the screen's edges, and the
      buttons still sit in the corner; minimise and restore from the taskbar.
      **(never seen working)**
- [ ] Switch VCMP's theme (Light, Dark, System) while it runs, and with System
      chosen switch Windows between light and dark: the window menu (Alt+Space)
      follows the theme. **(never seen working)**
- [ ] Settings → Zoom at 150 % and on a monitor scaled to 150 %: the buttons stay
      clickable where they are drawn, and the maximise button still shows Snap
      layouts. **(never seen working)**
- [ ] Narrator reads the buttons as Minimise, Maximise or Restore, and Close.
- [ ] The same for the zip that `packaging/release-local.sh` cross-builds with
      MinGW, and the rest of this list with it: it plays, and the media flyout,
      Credential Manager and the output devices work. **(seen only starting under
      Wine on Linux)**

## macOS (never compiled locally)

- [ ] It builds and runs.
- [ ] Now Playing in Control Center and the media keys show the track and control
      it; the cover shows. **(never seen working)**
- [ ] Cmd+click, Cmd+A and Cmd+F work where Ctrl does elsewhere.
- [ ] Claude Desktop with the entry Settings shows (`VCMP.app/Contents/MacOS/...`
      and `mcp`) lists vcmp's tools. **(never seen working)**
- [ ] Last.fm keeps its key in the Keychain across restarts.
- [ ] The `macos-arm64.zip` (or `macos-x64.zip`) unpacks to `VCMP.app`, which
      starts after System Settings → Privacy & Security → Open Anyway, and shows
      its icon.
