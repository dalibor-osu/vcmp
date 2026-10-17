//! The whole engine, driven through a test device that the test pulls audio from
//! in place of a sound card.

use std::f64::consts::TAU;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::*;
use crate::local::LocalSource;
use crate::output::testing::{DEFAULT_NAME, TestDevice};
use crate::player::Player;
use crate::queue::{QueueSlot, QueueSnapshot, RepeatMode};
use crate::source::{AudioChunk, TrackSource};
use crate::test_signal::{RATE, SPLIT, TOTAL, fixture, reference};
use crate::track::{SourceKind, TrackRef};

const TIMEOUT: Duration = Duration::from_secs(10);
const CD: AudioFormat = AudioFormat {
    sample_rate: RATE,
    channels: 2,
};
const DVD: AudioFormat = AudioFormat {
    sample_rate: 48_000,
    channels: 2,
};

/// Frames of the fade-in after a start or seek (10 ms), with a frame to spare.
fn ramp(format: AudioFormat) -> usize {
    format.sample_rate as usize / 100 + 1
}

/// Local files, plus made-up tracks: `synth:<rate>:<channels>:<seconds>` plays a
/// constant 0.25 for that long.
struct TestSource;

impl TrackSource for TestSource {
    fn open(&self, id: &str) -> Result<Box<dyn TrackStream>, PlaybackError> {
        let Some(spec) = id.strip_prefix("synth:") else {
            return LocalSource.open(id);
        };
        let parts: Vec<f64> = spec.split(':').map(|p| p.parse().unwrap()).collect();
        let format = AudioFormat {
            sample_rate: parts[0] as u32,
            channels: parts[1] as u16,
        };
        Ok(Box::new(Constant {
            format,
            total: (parts[0] * parts[2]) as usize,
            at: 0,
            samples: Vec::new(),
        }))
    }
}

struct Constant {
    format: AudioFormat,
    total: usize,
    at: usize,
    samples: Vec<f32>,
}

impl TrackStream for Constant {
    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.total as f64 / f64::from(self.format.sample_rate),
        ))
    }

    fn next_chunk(&mut self) -> Result<Option<AudioChunk<'_>>, PlaybackError> {
        let frames = (self.total - self.at).min(4096);
        if frames == 0 {
            return Ok(None);
        }
        self.at += frames;
        self.samples = vec![0.25; frames * usize::from(self.format.channels)];
        Ok(Some(AudioChunk {
            format: self.format,
            samples: &self.samples,
        }))
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, PlaybackError> {
        let rate = f64::from(self.format.sample_rate);
        self.at = ((position.as_secs_f64() * rate).round() as usize).min(self.total);
        Ok(Duration::from_secs_f64(self.at as f64 / rate))
    }
}

fn file(part: &str, extension: &str) -> QueueEntry {
    QueueEntry::new(TrackRef::local(fixture(part, extension).to_str().unwrap()))
}

fn synth(rate: u32, channels: u16, seconds: f64) -> QueueEntry {
    QueueEntry::new(TrackRef::local(format!(
        "synth:{rate}:{channels}:{seconds}"
    )))
}

fn missing(name: &str) -> QueueEntry {
    QueueEntry::new(TrackRef::local(format!("/nonexistent/vcmp/{name}.flac")))
}

struct Harness {
    player: Player,
    events: Receiver<Event>,
    device: TestDevice,
    format: AudioFormat,
    /// Every event so far, and whether a wait already matched it.
    seen: Vec<(Event, bool)>,
}

impl Harness {
    fn new(format: AudioFormat) -> Self {
        Harness::with(format, &[], None, |_| {})
    }

    fn with(
        format: AudioFormat,
        devices: &[&str],
        output_device: Option<&str>,
        before: impl FnOnce(&TestDevice),
    ) -> Self {
        let device = TestDevice::new(format, devices);
        before(&device);
        let options = PlayerOptions {
            sources: Sources::default().with(SourceKind::Local, Arc::new(TestSource)),
            output_device: output_device.map(str::to_owned),
            volume: 1.0,
        };
        let output = device.clone();
        let (player, events) = Player::with_output(options, move || output.output()).unwrap();
        Harness {
            player,
            events,
            device,
            format,
            seen: Vec::new(),
        }
    }

    /// Sends a command. Waits after it only match events that come later.
    fn send(&mut self, command: Command) {
        while let Ok(event) = self.events.try_recv() {
            self.seen.push((event, true));
        }
        for (_, matched) in &mut self.seen {
            *matched = true;
        }
        self.player.send(command);
    }

    fn play_context(&mut self, entries: Vec<QueueEntry>) {
        self.send(Command::PlayContext {
            entries,
            origin: None,
            start: None,
        });
    }

    /// Returns the earliest event that `want` accepts and no earlier wait matched,
    /// waiting for it if needed.
    fn wait_for(&mut self, what: &str, want: impl Fn(&Event) -> bool) -> Event {
        if let Some((event, matched)) = self
            .seen
            .iter_mut()
            .find(|(e, matched)| !*matched && want(e))
        {
            *matched = true;
            return event.clone();
        }
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(event) = self.events.recv_timeout(left) else {
                let seen: Vec<&Event> = self.seen.iter().map(|(e, _)| e).collect();
                panic!("no {what}; events so far: {seen:#?}");
            };
            let wanted = want(&event);
            self.seen.push((event.clone(), wanted));
            if wanted {
                return event;
            }
        }
    }

    fn count(&self, which: impl Fn(&Event) -> bool) -> usize {
        self.seen.iter().filter(|(e, _)| which(e)).count()
    }

    fn wait_for_track(&mut self, entry: Option<&QueueEntry>) {
        let what = format!("track change to {:?}", entry.map(|e| &e.track.id));
        self.wait_for(&what, |event| match event {
            Event::TrackChanged(now) => now.as_ref().map(|n| &n.entry) == entry,
            _ => false,
        });
    }

    fn wait_for_state(&mut self, state: PlaybackState) {
        self.wait_for(&format!("{state:?}"), |event| {
            *event == Event::StateChanged(state)
        });
    }

    fn render(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * usize::from(self.format.channels)];
        assert!(self.device.render(&mut out), "no stream running");
        out
    }

    /// Plays `frames` frames as a sound card would, waiting for the engine where
    /// the ring runs short.
    fn play(&self, frames: usize) -> Vec<f32> {
        let channels = usize::from(self.format.channels);
        let mut out = Vec::with_capacity(frames * channels);
        let deadline = Instant::now() + TIMEOUT;
        while out.len() < frames * channels {
            let ready = self
                .device
                .available()
                .min(frames - out.len() / channels)
                .min(1024);
            if ready == 0 {
                assert!(
                    Instant::now() < deadline,
                    "audio stopped after {} frames",
                    out.len() / channels
                );
                sleep(Duration::from_millis(1));
                continue;
            }
            out.extend(self.render(ready));
        }
        out
    }

    /// Plays until the engine has nothing more to give.
    fn play_to_end(&self) -> Vec<f32> {
        let mut out = Vec::new();
        let mut idle_since = Instant::now();
        while idle_since.elapsed() < Duration::from_millis(300) {
            let ready = self.device.available().min(1024);
            if ready == 0 {
                sleep(Duration::from_millis(1));
                continue;
            }
            out.extend(self.render(ready));
            idle_since = Instant::now();
        }
        out
    }

    /// Waits until the engine has written `frames` frames ahead of playback.
    fn wait_until_buffered(&self, frames: usize) {
        let deadline = Instant::now() + TIMEOUT;
        while self.device.available() < frames {
            assert!(
                Instant::now() < deadline,
                "only {} frames buffered",
                self.device.available()
            );
            sleep(Duration::from_millis(1));
        }
    }
}

/// Largest difference from the fixtures' reference signal, where `samples` start
/// at reference frame `first`; the first `skip` frames are left out.
fn max_error(samples: &[f32], first: usize, skip: usize) -> f32 {
    samples
        .chunks(2)
        .enumerate()
        .skip(skip)
        .flat_map(|(i, frame)| (0..2).map(move |c| (frame[c] - reference(first + i, c)).abs()))
        .fold(0.0, f32::max)
}

/// What plays between a jump and the new place: the old audio fading out, then at
/// most one callback of silence while the full ring makes room for the new audio.
fn assert_fade_then_silence(samples: &[f32]) {
    let frames = samples.len() / 2;
    assert!(
        frames <= ramp(CD) + 1024,
        "{frames} frames before the new place"
    );
    assert!(samples.iter().skip(ramp(CD) * 2).all(|&s| s == 0.0));
}

fn tolerance(extension: &str) -> f32 {
    match extension {
        "wav" | "flac" => 1e-4,
        "ogg" => 0.03,
        _ => 0.1,
    }
}

#[test]
fn an_album_plays_gaplessly_sample_for_sample() {
    for extension in ["wav", "flac", "mp3", "ogg"] {
        let mut h = Harness::new(CD);
        let (a, b) = (file("a", extension), file("b", extension));
        h.play_context(vec![a.clone(), b.clone()]);
        h.wait_for_track(Some(&a));
        let out = h.play_to_end();
        assert_eq!(
            out.len() / 2,
            TOTAL,
            "{extension}: every frame, nothing more"
        );
        let error = max_error(&out, 0, ramp(CD));
        assert!(error < tolerance(extension), "{extension}: error {error}");
        h.wait_for_track(Some(&b));
        h.wait_for_track(None);
        h.wait_for_state(PlaybackState::Stopped);
    }
}

#[test]
fn a_resampled_album_stays_one_continuous_signal() {
    let mut h = Harness::new(DVD);
    h.play_context(vec![file("a", "flac"), file("b", "flac")]);
    let out = h.play_to_end();
    assert_eq!(out.len() / 2, 48_000);
    // The tracks meet at frame 24 417; check across it, allowing the resampler's
    // constant sub-sample latency.
    let ideal =
        |frame: f64, channel: usize| 0.5 * (TAU * [440.0, 660.0][channel] * frame / 48_000.0).sin();
    let error = (-100..=100)
        .map(|k| {
            let shift = f64::from(k) / 100.0;
            (1_000..47_000)
                .flat_map(|i| (0..2).map(move |c| (i, c)))
                .map(|(i, c)| (out[2 * i + c] - ideal(i as f64 + shift, c) as f32).abs())
                .fold(0.0, f32::max)
        })
        .fold(f32::MAX, f32::min);
    assert!(error < 2e-3, "error {error}");
}

#[test]
fn a_format_change_between_tracks_keeps_every_frame() {
    let mut h = Harness::new(DVD);
    h.play_context(vec![
        synth(44_100, 2, 1.0),
        synth(48_000, 1, 0.5),
        synth(22_050, 6, 0.5),
    ]);
    let out = h.play_to_end();
    assert_eq!(out.len() / 2, 48_000 + 24_000 + 24_000);
    h.wait_for_track(None);
}

#[test]
fn play_next_replaces_the_prepared_track() {
    let mut h = Harness::new(CD);
    let (a, b, inserted) = (file("a", "flac"), file("b", "flac"), file("a", "wav"));
    h.play_context(vec![a.clone(), b.clone()]);
    // Both tracks are decoded into the ring before anything plays.
    h.wait_until_buffered(TOTAL);
    h.send(Command::PlayNext(vec![inserted.clone()]));
    h.wait_for(
        "queue change",
        |e| matches!(e, Event::QueueChanged(view) if view.up_next.len() == 1),
    );
    let out = h.play_to_end();
    assert_eq!(out.len() / 2, SPLIT + TOTAL);
    let (first, rest) = out.split_at(SPLIT * 2);
    let (second, third) = rest.split_at(SPLIT * 2);
    assert!(max_error(first, 0, ramp(CD)) < 1e-4);
    assert!(
        max_error(second, 0, 0) < 1e-4,
        "the inserted track plays in full"
    );
    assert!(
        max_error(third, SPLIT, 0) < 1e-4,
        "then the context continues"
    );
    h.wait_for_track(Some(&inserted));
    h.wait_for_track(Some(&b));
    h.wait_for_track(None);
}

#[test]
fn a_track_added_after_the_end_was_decoded_still_follows_gaplessly() {
    let mut h = Harness::new(CD);
    let (a, b) = (file("a", "flac"), file("b", "flac"));
    h.play_context(vec![a.clone()]);
    h.wait_until_buffered(SPLIT);
    h.send(Command::AddToQueue(vec![b.clone()]));
    let out = h.play_to_end();
    assert_eq!(out.len() / 2, TOTAL);
    assert!(max_error(&out, 0, ramp(CD)) < 1e-4);
    h.wait_for_track(Some(&b));
}

#[test]
fn seeking_continues_from_the_exact_sample() {
    let mut h = Harness::new(CD);
    h.play_context(vec![file("a", "flac")]);
    h.play(1000);
    h.send(Command::Seek(Duration::from_secs_f64(
        11_025.0 / f64::from(RATE),
    )));
    h.wait_for(
        "position",
        |e| matches!(e, Event::Position(p) if p.as_millis() == 250),
    );
    let out = h.play_to_end();
    let (old, new) = out.split_at(out.len() - (SPLIT - 11_025) * 2);
    assert_fade_then_silence(old);
    assert!(max_error(new, 11_025, ramp(CD)) < 1e-4);
    let announced = h.count(|e| matches!(e, Event::TrackChanged(Some(_))));
    assert_eq!(announced, 1, "a seek is not a new track");
}

#[test]
fn pause_keeps_the_place_and_play_resumes_there() {
    let mut h = Harness::new(CD);
    h.play_context(vec![file("a", "flac")]);
    let before = h.play(5_000);
    h.send(Command::Pause);
    h.wait_for_state(PlaybackState::Paused);
    let fading = h.render(2_000);
    let faded_out = h.device.taken() as usize - 5_000;
    assert!(faded_out <= ramp(CD), "{faded_out}");
    assert!(fading[faded_out * 2..].iter().all(|&s| s == 0.0));
    h.render(2_000);
    assert_eq!(
        h.device.taken() as usize,
        5_000 + faded_out,
        "nothing plays while paused"
    );
    h.send(Command::TogglePlayPause);
    h.wait_for_state(PlaybackState::Playing);
    let after = h.play_to_end();
    assert_eq!(before.len() / 2 + faded_out + after.len() / 2, SPLIT);
}

#[test]
fn next_and_previous_move_through_the_queue() {
    let mut h = Harness::new(CD);
    let tracks = vec![
        synth(44_100, 2, 1.0),
        synth(44_100, 2, 10.0),
        synth(44_100, 2, 1.5),
    ];
    h.play_context(tracks.clone());
    h.wait_for_track(Some(&tracks[0]));
    h.send(Command::Next);
    h.wait_for_track(Some(&tracks[1]));
    h.send(Command::Next);
    h.wait_for_track(Some(&tracks[2]));
    h.send(Command::Previous);
    h.wait_for_track(Some(&tracks[1]));
    // Past the threshold, Previous restarts the track without announcing a new one.
    h.play(RATE as usize * 4);
    h.send(Command::Previous);
    h.wait_for("restart", |e| *e == Event::Position(Duration::ZERO));
    h.send(Command::Next);
    h.wait_for_track(Some(&tracks[2]));
    assert_eq!(h.count(|e| matches!(e, Event::TrackChanged(_))), 5);
    h.send(Command::Next);
    h.wait_for_track(None);
    h.wait_for_state(PlaybackState::Stopped);
}

#[test]
fn repeat_one_plays_the_track_again_without_a_gap() {
    let mut h = Harness::new(CD);
    let a = file("a", "flac");
    h.send(Command::SetRepeat(RepeatMode::One));
    h.play_context(vec![a.clone()]);
    let out = h.play(3 * SPLIT);
    for (round, part) in out.chunks(SPLIT * 2).enumerate() {
        let skip = if round == 0 { ramp(CD) } else { 0 };
        assert!(max_error(part, 0, skip) < 1e-4, "round {round}");
    }
    for _ in 0..3 {
        h.wait_for_track(Some(&a));
    }
}

#[test]
fn unplayable_tracks_are_reported_and_skipped() {
    let mut h = Harness::new(CD);
    let (a, b) = (file("a", "flac"), file("b", "flac"));
    h.play_context(vec![
        missing("first"),
        a.clone(),
        missing("between"),
        b.clone(),
    ]);
    let out = h.play_to_end();
    assert_eq!(
        out.len() / 2,
        TOTAL,
        "the playable tracks still join gaplessly"
    );
    assert!(max_error(&out, 0, ramp(CD)) < 1e-4);
    h.wait_for_track(None);
    let failed: Vec<&str> = h
        .seen
        .iter()
        .filter_map(|(e, _)| match e {
            Event::TrackFailed { track, error } => {
                assert!(matches!(
                    error,
                    PlaybackError::Io(std::io::ErrorKind::NotFound, _)
                ));
                Some(track.id.as_ref())
            }
            _ => None,
        })
        .collect();
    assert_eq!(failed.len(), 2);
    assert!(failed[0].ends_with("first.flac") && failed[1].ends_with("between.flac"));
}

#[test]
fn a_queue_of_only_missing_tracks_stops() {
    let mut h = Harness::new(CD);
    h.send(Command::SetRepeat(RepeatMode::All));
    h.play_context(vec![missing("one"), missing("two")]);
    h.wait_for_track(None);
    h.wait_for_state(PlaybackState::Stopped);
}

#[test]
fn stop_and_play_after_the_end_start_from_the_beginning() {
    let mut h = Harness::new(CD);
    let a = file("a", "wav");
    h.play_context(vec![a.clone()]);
    h.play(1_000);
    h.send(Command::Stop);
    h.wait_for_state(PlaybackState::Stopped);
    h.send(Command::Play);
    h.wait_for_state(PlaybackState::Playing);
    let out = h.play_to_end();
    let (old, new) = out.split_at(out.len() - SPLIT * 2);
    assert_fade_then_silence(old);
    assert!(max_error(new, 0, ramp(CD)) < 1e-4, "from the start again");
    h.wait_for_track(None);
    h.send(Command::Play);
    h.wait_for_track(Some(&a));
    assert_eq!(h.play_to_end().len() / 2, SPLIT);
}

#[test]
fn restore_loads_the_queue_paused_at_the_saved_position() {
    let mut h = Harness::new(CD);
    let (a, b) = (file("a", "flac"), file("b", "flac"));
    let mut queue = Queue::with_seed(1);
    queue.set_context(vec![a.clone(), b.clone()], Some("album:7".into()), None);
    queue.add_to_queue([file("a", "wav")]);
    let snapshot = PlaybackSnapshot {
        queue: queue.snapshot(),
        position: Duration::from_secs_f64(11_025.0 / f64::from(RATE)),
    };
    h.send(Command::Restore(snapshot.clone()));
    h.wait_for_state(PlaybackState::Paused);
    h.wait_for_track(Some(&a));
    h.wait_for("restored position", |e| {
        *e == Event::Position(snapshot.position)
    });
    assert_eq!(
        h.player.snapshot().unwrap(),
        snapshot,
        "nothing moved while paused"
    );
    h.render(1_000);
    assert_eq!(h.player.snapshot().unwrap().position, snapshot.position);
    h.send(Command::Play);
    h.wait_for_state(PlaybackState::Playing);
    let out = h.play(SPLIT - 11_025);
    assert!(max_error(&out, 11_025, ramp(CD)) < 1e-4);
}

#[test]
fn a_broken_snapshot_is_rejected() {
    let mut h = Harness::new(CD);
    let snapshot = PlaybackSnapshot {
        queue: QueueSnapshot {
            current: Some(QueueSlot::Context(3)),
            ..QueueSnapshot::default()
        },
        position: Duration::ZERO,
    };
    h.send(Command::Restore(snapshot));
    h.wait_for("rejection", |e| {
        *e == Event::RestoreFailed(PlaybackError::InvalidSnapshot)
    });
}

#[test]
fn snapshots_report_the_position_while_playing() {
    let mut h = Harness::new(CD);
    h.play_context(vec![file("a", "flac"), file("b", "flac")]);
    h.play(11_025);
    let snapshot = h.player.snapshot().unwrap();
    assert_eq!(snapshot.position, Duration::from_millis(250));
    assert_eq!(snapshot.queue.context.len(), 2);
}

#[test]
fn the_output_device_is_chosen_by_name_with_a_fallback() {
    let mut h = Harness::with(CD, &["Speakers", "Headphones"], Some("Headphones"), |_| {});
    h.wait_for("headphones", |e| {
        *e == Event::OutputChanged(OutputInfo {
            device: "Headphones".into(),
            format: CD,
            fell_back: false,
        })
    });
    h.send(Command::SetOutputDevice(Some("Unplugged USB DAC".into())));
    h.wait_for("fallback", |e| {
        *e == Event::OutputChanged(OutputInfo {
            device: DEFAULT_NAME.into(),
            format: CD,
            fell_back: true,
        })
    });
}

#[test]
fn a_lost_device_is_reopened_and_playback_carries_on() {
    let mut h = Harness::new(CD);
    h.wait_for("first output", |e| matches!(e, Event::OutputChanged(_)));
    h.play_context(vec![file("a", "flac")]);
    h.play(5_000);
    h.device.unplug();
    h.wait_for("reopened output", |e| matches!(e, Event::OutputChanged(_)));
    let out = h.play_to_end();
    assert_eq!(out.len() / 2, SPLIT - 5_000);
    assert!(max_error(&out, 5_000, ramp(CD)) < 1e-4);
}

#[test]
fn without_a_device_commands_still_work_and_play_retries() {
    let mut h = Harness::with(CD, &[], None, |device| device.break_down(true));
    h.wait_for("output failure", |e| matches!(e, Event::OutputFailed(_)));
    let a = file("a", "flac");
    h.play_context(vec![a.clone()]);
    h.wait_for_track(Some(&a));
    h.wait_for_state(PlaybackState::Paused);
    h.device.break_down(false);
    h.send(Command::Play);
    h.wait_for("output", |e| matches!(e, Event::OutputChanged(_)));
    h.wait_for_state(PlaybackState::Playing);
    assert_eq!(h.play_to_end().len() / 2, SPLIT);
}

#[test]
fn volume_follows_a_cubic_curve() {
    let mut h = Harness::new(CD);
    h.send(Command::SetVolume(0.5));
    h.play_context(vec![synth(44_100, 2, 1.0)]);
    let out = h.play(5_000);
    assert!(
        out[ramp(CD) * 2..]
            .iter()
            .all(|&s| (s - 0.25 * 0.125).abs() < 1e-6)
    );
}

#[test]
fn the_queue_panel_sees_every_change() {
    let mut h = Harness::new(CD);
    let tracks: Vec<QueueEntry> = (0..4).map(|_| synth(44_100, 2, 1.0)).collect();
    h.send(Command::PlayContext {
        entries: tracks.clone(),
        origin: Some("playlist:3".into()),
        start: Some(1),
    });
    h.send(Command::AddToQueue(vec![synth(48_000, 2, 1.0)]));
    let view = h.wait_for(
        "up next",
        |e| matches!(e, Event::QueueChanged(v) if v.up_next.len() == 1),
    );
    let Event::QueueChanged(view) = view else {
        unreachable!()
    };
    assert_eq!(view.origin.as_deref(), Some("playlist:3"));
    assert_eq!(view.upcoming.len(), 2);
    h.send(Command::SkipTo(1));
    let view = h.wait_for(
        "skip",
        |e| matches!(e, Event::QueueChanged(v) if v.up_next.is_empty()),
    );
    let Event::QueueChanged(view) = view else {
        unreachable!()
    };
    assert_eq!(view.current.as_ref(), Some(&tracks[2]));
}
