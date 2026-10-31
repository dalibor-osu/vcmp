# Playback engine

How `crates/vcmp-playback` turns files into sound, and why it is built this way. The
architecture comes from the [v1 specification](specs/2026-10-17-vcmp-music-player.md)
(workstream 3); this file records how the code applies it. Keep it in sync with the
crate.

## Using it

The crate has no UI dependency. The UI starts a `Player`, sends it `Command`s and reads
`Event`s from the channel `Player::new` returns. It shares no mutable state with the
engine.

```rust
let (player, events) = Player::new(PlayerOptions::default())?;
player.send(Command::PlayContext { entries, origin: Some("album:42".into()), start: Some(3) });
// events: StateChanged, TrackChanged, Position, QueueChanged, TrackFailed,
//         OutputChanged, OutputFailed, RestoreFailed, Decoding, StopAfterCurrent,
//         Synced
```

- **Events into Slint (workstream 4):** forward them from a thread blocking on the
  receiver, through `slint::invoke_from_event_loop`.
- **Device list:** `output_devices()` lists output devices by name for Settings. On
  Linux these are the sound server's outputs, named as in the desktop's sound
  settings, or ALSA's devices without its converter and utility plugins when no
  server answers (see [Linux: PulseAudio, then ALSA](#linux-pulseaudio-then-alsa)).
  The system default is `None`.
- **Queue entries:** each entry is a `TrackRef` (source plus id; a local file's id is
  its path) and optional artist and album keys.
  - Shuffle uses the keys to keep the same artist or album apart.
  - Any value unique per artist and per album works, such as a database id.
- **Playable formats:** MP3, FLAC, WAV and Ogg Vorbis. Opus is not supported (see the
  spec's risk table).
- **Waiting for a command's effect:** `Command::Sync(token)` is answered with
  `Event::Synced(token)` once the commands sent before it are done. Events come in
  order, so whoever sees `Synced` has seen what those commands caused. The app
  uses it to answer AI agents only once the window shows the result of their
  request (see [mcp.md](mcp.md#changes-go-through-the-app)).
- **Open files:** `Event::Decoding` names the track whose file the engine has open
  (see [Open files](#open-files)). The tag editor uses it on Windows, which can't
  replace an open file.

## Threads

| Thread | Runs | May block |
|---|---|---|
| Caller (UI) | `Player::send`, `Player::snapshot` | Briefly, in `snapshot` only |
| `vcmp-playback` | Owns the queue, decoder, converter and output stream. Writes the ring and sends events | Yes: file I/O, decoding |
| Audio callback (cpal) | `Renderer::render`: reads the ring, applies gain, writes the device buffer | **Never** |
| `vcmp-output-watch` (PulseAudio only) | Checks twice a second that the stream plays on the device it should (see [Output](#output)) | Yes: requests to the sound server |

**The callback rule.** The callback never allocates, locks, waits or does I/O. It only
touches two rtrb queues (wait-free) and a few atomics:

- frames played;
- the paused flag;
- the gain.

**How the rule is enforced.**

- **Allocations:** the test build installs a counting global allocator
  (`src/alloc_counter.rs`). `ring::tests::no_allocation_in_the_callback` then renders
  through every state with the counter watching:
  - playing;
  - a volume change;
  - pause and resume;
  - a flush and a cut;
  - an underrun;
  - `f32`, `i16` and `u8` output.

  A single allocation or free fails the test.
- **Locks:** these are kept out by construction. The callback type holds nothing that
  can lock.

**Around the callback with PulseAudio.** The rule covers `Renderer::render`, not
the backend around it. With PulseAudio the callback runs on the client's socket
thread whenever the server asks for audio. That thread is not real-time and holds
the client's mutex during the callback. Afterwards, cpal wakes its own latency
thread through a mutex and a condition variable. Its latency queries allocate a
little on the socket thread when their replies come in. This is acceptable here
because the deadline is not the sound card's interrupt. The server keeps two periods
(about 43 ms) buffered and asks for the next period when one has played, so the
client has about 21 ms to answer. Measured in an optimised build, a stream on
PipeWire had no underruns in 15 s with 64 busy threads on 16 cores (4× overload).

## Data path

```text
TrackSource ─► TrackStream ─► Converter ─────────────► ring (rtrb, 2 s) ─► Renderer ─► device
 (Local:        decoded f32,   channels, then           frames numbered    gain, fades,
  symphonia)    delay/padding  sample rate (rubato)     from 0 since the   skips; never
                trimmed                                 output opened      allocates
```

1. **Sources.** A source (`TrackSource`) opens a track id and returns a `TrackStream`
   of decoded, interleaved `f32` chunks. It removes encoder delay and padding itself,
   and seeks to an exact sample. `LocalSource` does this for files with symphonia.
   - A future Spotify source plugs in here, behind the same queue, shuffle and output.
2. **Converter.** It maps channels to the device's layout, then resamples to the
   device's rate. Consecutive tracks of the same format form one run: they pass through
   the same resampler with no break, so there is no seam where they meet. When the
   format changes, the run is flushed. Each run produces exactly
   `round(input frames × ratio)` frames, so track boundaries stay at exact frame
   numbers.
3. **Ring.** Frames are numbered by how many were ever written. A track in the ring is a
   *segment* starting at a frame number. When the callback's frame counter passes the
   next segment's start, that track has become audible. The engine then advances the
   queue and sends `TrackChanged`.
4. **Jumps** (seek, skip, a new context) send `Control::Flush`. The callback fades out
   for 10 ms, then drops everything written before the flush, and the new audio fades
   in.
5. **Queue edits after the next track was decoded** ("Play next" during the last
   seconds of a track) send `Control::Cut`. At the boundary the callback jumps over the
   stale next track to its replacement, without a fade. Asking to stop after the
   current track cuts the next track the same way, with nothing after it.

## Gapless playback

- **The stream stays open.** One output stream stays open across tracks.
- **Decoding ahead.** When the current track has been decoded to its end, the engine
  opens the next one at once and writes its frames straight after. Only one track waits
  ahead; the one after it is opened once that track becomes audible.
- **Trimming.** Each format declares its encoder delay and padding differently:

| Format | Where delay and padding come from |
|---|---|
| MP3 | The LAME/Xing (or `Lavf`/`Lavc`) header, read by symphonia. iTunes' `iTunSMPB` tag is not read |
| Ogg Vorbis | Ogg granule positions |
| FLAC, WAV | Nothing to trim |

Tests decode two halves of one tone, encoded separately in every format. They check:

- **Length:** the joined halves have exactly the original sample count.
- **Waveform:** the joined halves match the uninterrupted waveform.
- **Through the engine:** the same holds when played through the engine, at the
  source rate and resampled to 48 kHz.

## Open files

The engine keeps one file open at a time: the track it decodes.

| When | The open file |
|---|---|
| A track starts, or is restored paused | Its file is opened |
| Playing or paused | Stays open |
| Seeking | Closed and opened again at once; not reported |
| The track is decoded to its end, about a ring's length (2 s) before it ends | Closed. The next track's file opens to follow gaplessly |
| Skipping, a new context, Stop, the end of the queue | Closed (and the new track's opened) |

`Event::Decoding(Option<TrackRef>)` is sent when the open track changes. The engine
checks once per round of its loop, after commands and decoding, so the event comes
after the old file was closed, and a seek's reopening of the same file is not sent.
Releasing the file while paused (and opening it again at the decoded position on
resume) would let tag edits through sooner, but isn't built.

## Output

| Behaviour | How |
|---|---|
| Device selection | By name, from `output_devices()`. `None` is the system default |
| Missing device | Falls back to the system default. `OutputChanged { fell_back: true }` lets the UI tell the user |
| Format | The device's default config, switched to stereo when the device offers it. Any cpal sample format (`f32`, `i16`, `i24`, `u8`...) is converted in the callback |
| Unplugged device or invalidated stream | The engine reopens the output and resumes at the same position |
| No device at all | `OutputFailed`; playback pauses, the queue keeps working, and Play retries |
| Volume | `SetVolume(0.0..=1.0)` is a slider position. The amplitude is its cube, which is close to how loudness is heard. Changes ramp over 10 ms |

### Linux: PulseAudio, then ALSA

cpal's `pulseaudio` feature is on. When a PulseAudio server answers, the output goes
through it. That is PipeWire's pipewire-pulse on most desktops, or PulseAudio itself.
The client is the pure-Rust `pulseaudio` crate, so nothing is needed at build time
and libpulse isn't needed at run time. Otherwise, the output is ALSA.

| Behaviour | How |
|---|---|
| Choosing the backend | cpal's `default_host()` tries PulseAudio first: `$PULSE_SERVER` (a `unix:` path) or `$XDG_RUNTIME_DIR/pulse/native`, giving up after 2 s. One host serves the whole run (`host()` in `src/output.rs`). Each open checks it: a dead PulseAudio connection (the server stopped or restarted) is replaced, possibly by ALSA, and ALSA is replaced once a server is there again |
| Device list | The server's outputs (sinks) by description, such as "Scarlett Solo (3rd Gen.) Headphones / Line 1-2". Any of them opens while other apps play, and mixes with them |
| Buffer | A fixed period of 1024 frames at 48 kHz, scaled to the device's rate. The server holds two, so seek, pause and volume are heard 25 to 45 ms later. With cpal's `BufferSize::Default` the server would buffer 2 s |
| Device changes | cpal reports neither moves nor default changes. A `vcmp-output-watch` thread therefore looks up the device `open` would pick, twice a second. When that is another device, it marks the stream lost and the engine reopens. This covers three cases: the default changed, the chosen device went (the server has already moved the sound to the default; now Settings says so), or the chosen device came back (playback returns to it) |
| Server gone | The stream reports a lost connection and the engine reopens, through ALSA if no server answers |

With ALSA, the device list is ALSA's PCMs without the utility plugins. A card opens
directly, past any sound server (see Known limits).

## Channels

| Source → device | Mapping |
|---|---|
| Same count | Unchanged |
| Mono → stereo or more | Mono on front left and right |
| Stereo → mono | Average |
| 3–8 channels → stereo | ITU-style downmix with −3 dB centre and surrounds, LFE dropped, scaled so it cannot clip. Assumes the standard WAVE/FLAC channel order |
| Surround → other surround | The shared leading channels are copied, the rest are silent |
| More than 8 channels → stereo | The first two channels |

## Queue

These are the agreed semantics. The pure logic is `Queue`, and the engine drives it.

- **Context.** Playing an album, playlist or list makes it the *context*.
  - `origin` is the app's own key for it ("album:42"), handed back in `QueueView`.
  - A new context clears the history.
- **Up next.** "Play next" puts tracks at the front of Up next, and "Add to queue"
  appends them. Up next always plays before the context continues, and survives a new
  context.
- **Previous.**
  - It restarts the current track when more than 3 s (`RESTART_THRESHOLD`) of it has
    played, or when there is no history.
  - Otherwise it goes back. The track left behind returns to where it came from: the
    front of Up next, or the front of the context.
- **Repeat: off, all or one.** Repeat one replays a track when it ends. Next still
  moves on, and the context wraps around. Repeat is not in the spec; it was added
  because "each track once per cycle" needs a defined end of a cycle.
- **End of the queue.** The player stops, and Play starts the context over.
- **Stop after the current track** (`Command::StopAfterCurrent`, the sleep timer's
  end). Nothing is prepared after the current track, and a next track already in the
  ring is cut. When the track ends, the queue moves on as usual and the player stops
  with the next track waiting at its start. `TrackChanged` names that track with no
  duration, since its file opens only when it plays.
  - Skipping keeps the request for whichever track is current then; Repeat one stops
    instead of repeating.
  - It is cleared once it has stopped, by Stop, and when the queue runs out;
    `Event::StopAfterCurrent` reports each change.
  - Doing this in the app on `TrackChanged` would let a moment of the next track
    through, since that event comes once the next track is audible.
- **Unplayable tracks** (missing file, unplugged drive, unsupported data) are reported
  with `TrackFailed` and skipped. After 100 in a row the player stops.

### Shuffle

Shuffle applies to the context only. Up next keeps its order.

**True shuffle.** Every next track is an independent uniform pick from the whole
context, so a track can come back soon or even straight away, and playback never runs
out. Picks are drawn one ahead and then fixed, so what the queue panel shows is what
plays.

**Less repeats.** Each cycle is a permutation: every track plays once. The order keeps
the same artist and the same album out of adjacent slots where the context allows it.

- **Artists:** adjacent same-artist pairs are always the minimum possible, including the
  pair with the track before the cycle.
- **Albums:**
  - Among the choices that keep that minimum, the one that also keeps same-album pairs
    lowest is taken, one step at a time.
  - This is exact when the artists leave the albums free (one artist, or all artists
    different) and a heuristic otherwise.
- **Unknown keys:** a track with an unknown artist or album only matches entries of the
  same track. A duplicated playlist entry still counts as a repeat.

How it works:

- **Construction.** Tracks are placed one at a time. Only a group holding about half of
  what is left can force conflicts.
- **Each step.** The step computes, in closed form, the fewest conflicts still
  unavoidable after each candidate, and picks at random among the cheapest.
- **Speed.** While no group is that large, any track differing in both keys is optimal
  and is drawn directly. Ordering 20 000 tracks takes about 10 ms.

**Turning shuffle on or off.**

- **Turned off:** play continues in context order after the current track.
- **Less repeats turned on:** a new cycle of every track except the current one starts.

**Tests.** `shuffle::tests` checks these guarantees exhaustively against brute force, for
every grouping of up to 6 tracks.

## Saving the queue across restarts

The owner's decision (2026-10-17): the queue and position come back after a restart,
paused.

- **Saving.** `Player::snapshot()` returns a `PlaybackSnapshot`: the context, order,
  history, Up next, shuffle and repeat modes, and the position. It is plain data.
  The app stores it as versioned JSON in the library's settings
  (`crates/vcmp-app/src/saved_queue.rs`) whenever a track starts and on exit.
- **Restoring.** `Command::Restore(snapshot)` loads it paused at the saved position.
  - A snapshot whose indices do not fit is rejected with `RestoreFailed`.
  - A saved track whose file has gone is skipped like any unplayable track.

## Files

| File | Holds |
|---|---|
| `src/player.rs` | `Player`, `Command`, `Event` and the other public types |
| `src/engine.rs` | The engine thread: commands, decoding, segments, events |
| `src/queue.rs` | `Queue`: context, Up next, history, repeat, snapshots |
| `src/shuffle.rs` | The "less repeats" order |
| `src/source.rs`, `src/local.rs` | `TrackSource`, `TrackStream` and the symphonia-based `LocalSource` |
| `src/convert.rs` | Channel mapping and resampling |
| `src/ring.rs` | The ring buffer, `Control`s and `Renderer` (the callback) |
| `src/output.rs` | cpal devices and stream, the shared host, the PulseAudio device watch, plus the test device |
| `src/alloc_counter.rs` | The counting allocator used by the no-allocation test |
| `tests/fixtures/` | Tone fixtures and `make-fixtures.sh`, which regenerates them with ffmpeg |
| `examples/play.rs` | Plays files on the real device and prints events |

## Design decisions

| Decision | Reason |
|---|---|
| One engine thread does commands, decoding and conversion | A single owner of the queue and decoder state needs no locks. Decoding a packet takes well under the 5 ms polling interval, so commands stay responsive |
| 2 s ring | Rides out a slow disk (the owner's library is on an HDD) and a busy system. Seeks and skips drop buffered audio, so a large ring adds no latency |
| Frame numbers, not track markers, in the ring | The callback stays ignorant of tracks. Boundaries, cuts and flushes are all one comparison |
| rubato's FFT resampler with fixed input and output chunks | The best quality the crate offers for fixed ratios, and allocation-free per call. It adds a constant latency of half an input sample, which is harmless |
| Seeks start 4096 samples early and decode up to the target | Right after a reset, the MP3 decoder lacks the previous frame's overlap. The first frame would be distorted at the target |
| Errors carry English `Display` text | Meant for logs. The UI shows its own translated text per `PlaybackError` variant |
| PulseAudio on Linux, through cpal, with ALSA as the fallback (2026-10-18) | Through ALSA, a sound card opened directly fails as busy while PipeWire uses it, and the device list is ALSA's plugins. cpal's PulseAudio backend lists the server's outputs and mixes with other apps. It is pure Rust, so nothing new is needed at build time, and pipewire-pulse serves it too. cpal's PipeWire backend would need libpipewire's headers to build |
| A 1024-frame period at 48 kHz | PipeWire's default quantum. A 960-frame period already lowered the graph's quantum to 512 (measured with `pw-top`), which wakes every app's audio twice as often. On real hardware the latency measured 27 ms, against 16 ms for the ALSA path's 512-frame periods |
| One shared host | Each dropped PulseAudio host leaves its connection and a thread behind (measured: five hosts, five threads, five server clients), and Settings lists the devices every time it opens |
| Polling for device changes, off the engine thread | The `pulseaudio` crate ignores the server's "stream moved" and "stream killed" messages, and cpal has no device events. A request to a server that stops answering would block, and the engine thread must keep answering `snapshot` |
| Reopening on a default change, also on PipeWire | PipeWire moves the stream to the new default by itself: cpal opens the default by its index, and WirePlumber moves streams that pointed at the old default. PulseAudio keeps such a stream where it is, so VCMP reopens on both |

## Known limits

- **MP3 gapless:** an MP3 without a LAME/Xing header, or with only iTunes' `iTunSMPB`,
  plays with its encoder delay and padding, so with a small gap.
- **Linux without a PulseAudio server uses ALSA:** Settings lists "System default"
  first, then the sound servers ALSA reaches (PipeWire, JACK) and the cards' own
  devices. `output_devices` leaves out the plugins that convert or discard audio
  (rate converters, up- and downmix, speex, null), the OSS and USB stream plugins,
  which almost never open, and ALSA's own names for the default. A card is opened
  directly, past any sound server: while one holds it, opening fails as busy and
  the player shows that as a problem. A saved device that is now hidden still opens.
- **PulseAudio device names:** the saved name is the output's description, which is
  what Settings shows. Two outputs with the same description (two identical USB
  interfaces) are one entry, and the first one opens. A description that changes
  counts as another device. Examples: an HDMI output named after the monitor
  plugged in, a renamed Bluetooth device, another card profile. A name saved while
  VCMP used ALSA ("Scarlett Solo USB, USB Audio") is no longer listed, so the default
  plays and Settings says so until another device is chosen.
- **A reopen skips a little:** reopening drops what the server still holds (up to
  43 ms) and takes a few milliseconds. On PipeWire this happens up to half a second
  after the server has already moved the sound to a new default.
- **How VCMP shows up in the mixer:** with PulseAudio, the per-app volume controls of
  the desktop list VCMP as `cpal-pulseaudio-<pid>`. cpal sets that client name and
  gives the stream no name or icon.
- **`PULSE_RUNTIME_PATH`:** the `pulseaudio` crate looks for the socket in
  `$PULSE_RUNTIME_PATH/pulse/native`, but libpulse uses `$PULSE_RUNTIME_PATH/native`.
  The crate also gives up at the first missing path, so with that variable set VCMP
  uses ALSA. `PULSE_SERVER` is used only for a `unix:` path.
- **A PulseAudio server that stops answering:** connecting gives up after 2 s, but
  later requests have no timeout. A server that keeps the connection open and never
  answers blocks the engine thread while it opens the output.
- **Unoptimised `pulseaudio` crate:** the crate zero-fills a 1 MB buffer before each
  socket read. Unoptimised, that took about 20 ms per read under load and caused
  constant underruns. The dev profile optimises dependencies
  (`[profile.dev.package."*"]`); keep it that way.
- **No return to the requested device with ALSA (and on Windows and macOS):** after
  a fallback, the engine stays on the default even if the requested device comes
  back. It switches on the next `SetOutputDevice` or restart. With PulseAudio it
  returns by itself.
- **Silence after a seek:** with a full ring, the new audio can arrive one callback
  period after the fade-out, which leaves a few milliseconds of silence.
- **Replaced next track while resampling:** if the track that was to follow is replaced
  while resampling is active, the old track's tail and the replacement are resampled
  separately. That join is not seamless.
- **Very short tracks:** only one track is decoded ahead. A track shorter than the time
  needed to open the one after it (a few milliseconds) can leave a gap.
- **Chained Ogg files** (several streams in one file) play up to the first stream
  change.
- **Paths:** track ids must be valid UTF-8.
