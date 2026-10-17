//! The engine thread: owns the queue, decodes, converts and feeds the ring, follows
//! what the callback has played, and turns commands into all of that.
//!
//! The ring holds the current track and, once its decoding is done, the start of the
//! next one written straight after it (gapless). Each track in the ring is a
//! [`Segment`] starting at a frame number; when the callback's read position passes
//! the next segment's start, that track has become audible and the queue moves on.
//! If the queue changes so that a different track should follow, the prepared one
//! is cut out of the ring (see [`Control::Cut`]); jumps (seek, skip, a new context)
//! flush the ring instead.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::convert::Converter;
use crate::error::PlaybackError;
use crate::output::Output;
use crate::player::{
    Command, Event, NowPlaying, OutputInfo, PlaybackSnapshot, PlaybackState, PlayerOptions,
    RESTART_THRESHOLD,
};
use crate::queue::{Advance, Queue};
use crate::ring::{self, Control, Feed};
use crate::source::{AudioFormat, Sources, TrackStream};
use crate::track::QueueEntry;

pub(crate) enum Message {
    Command(Command),
    Snapshot(Sender<PlaybackSnapshot>),
    Shutdown,
}

/// Audio the ring holds: room to ride out slow disks and a busy system.
const RING_SECONDS: f32 = 2.0;
/// How often to check progress while playing, and while not.
const POLL_PLAYING: Duration = Duration::from_millis(5);
const POLL_IDLE: Duration = Duration::from_millis(50);
const POSITION_INTERVAL: Duration = Duration::from_millis(250);
/// Unplayable tracks skipped in a row before giving up.
const MAX_FAILURES: usize = 100;
/// The ring's format while no device is open, so the queue keeps working.
const NO_DEVICE: AudioFormat = AudioFormat {
    sample_rate: 48_000,
    channels: 2,
};

/// A track in the ring.
struct Segment {
    entry: QueueEntry,
    /// Ring frame where it starts.
    start: u64,
    /// Track position at `start` (after a seek or restore).
    offset: Duration,
    duration: Option<Duration>,
}

/// Amplitude for a volume slider position: a cubic curve, close to how loudness is
/// heard.
fn gain(volume: f32) -> f32 {
    volume.clamp(0.0, 1.0).powi(3)
}

pub(crate) struct Engine {
    messages: Receiver<Message>,
    events: Sender<Event>,
    sources: Sources,
    queue: Queue,
    output: Box<dyn Output>,
    device: Option<String>,
    connected: bool,
    format: AudioFormat,
    feed: Feed,
    converter: Converter,
    /// Converted samples waiting for room in the ring; `pending[pending_at..]` is
    /// left, and its first frame will be ring frame `feed.written()`.
    pending: Vec<f32>,
    pending_at: usize,
    /// Controls the callback's queue had no room for yet.
    backlog: VecDeque<Control>,
    /// Ring frame of the converter's output frame 0.
    stream_base: u64,
    decoder: Option<Box<dyn TrackStream>>,
    segments: VecDeque<Segment>,
    /// Ring frame where the queue runs out.
    end_at: Option<u64>,
    state: PlaybackState,
    volume: f32,
    failures: usize,
    last_position: Instant,
}

impl Engine {
    pub(crate) fn new(
        messages: Receiver<Message>,
        events: Sender<Event>,
        options: PlayerOptions,
        output: Box<dyn Output>,
    ) -> Self {
        let (feed, _) = ring::channel(NO_DEVICE, RING_SECONDS, gain(options.volume), false);
        Engine {
            messages,
            events,
            sources: options.sources,
            queue: Queue::new(),
            output,
            device: options.output_device,
            connected: false,
            format: NO_DEVICE,
            feed,
            converter: Converter::new(NO_DEVICE),
            pending: Vec::new(),
            pending_at: 0,
            backlog: VecDeque::new(),
            stream_base: 0,
            decoder: None,
            segments: VecDeque::new(),
            end_at: None,
            state: PlaybackState::Stopped,
            volume: options.volume,
            failures: 0,
            last_position: Instant::now(),
        }
    }

    pub(crate) fn run(mut self) {
        self.open_output();
        loop {
            loop {
                match self.messages.try_recv() {
                    Ok(message) => {
                        if !self.receive(message) {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if self.output.lost() {
                self.reopen();
            }
            self.send_backlog();
            self.follow_playback();
            if !self.fill() {
                let wait = if self.state == PlaybackState::Playing {
                    POLL_PLAYING
                } else {
                    POLL_IDLE
                };
                match self.messages.recv_timeout(wait) {
                    Ok(message) => {
                        if !self.receive(message) {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    }

    fn receive(&mut self, message: Message) -> bool {
        match message {
            Message::Command(command) => self.handle(command),
            Message::Snapshot(reply) => {
                let _ = reply.send(PlaybackSnapshot {
                    queue: self.queue.snapshot(),
                    position: self.position(),
                });
            }
            Message::Shutdown => return false,
        }
        true
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::PlayContext {
                entries,
                origin,
                start,
            } => {
                self.queue.set_context(entries, origin, start);
                self.failures = 0;
                self.start_playing();
            }
            Command::PlayNext(entries) => {
                self.queue.play_next(entries);
                self.queue_changed();
            }
            Command::AddToQueue(entries) => {
                self.queue.add_to_queue(entries);
                self.queue_changed();
            }
            Command::RemoveFromUpNext(index) => {
                self.queue.remove_up_next(index);
                self.queue_changed();
            }
            Command::MoveInUpNext { from, to } => {
                self.queue.move_up_next(from, to);
                self.queue_changed();
            }
            Command::ClearUpNext => {
                self.queue.clear_up_next();
                self.queue_changed();
            }
            Command::SetShuffle(mode) => {
                self.queue.set_shuffle(mode);
                self.queue_changed();
            }
            Command::SetRepeat(mode) => {
                self.queue.set_repeat(mode);
                self.queue_changed();
            }
            Command::SkipTo(index) => {
                self.queue.skip_to(index);
                self.start_playing();
            }
            Command::Play => self.play(),
            Command::Pause => {
                if self.state == PlaybackState::Playing {
                    self.set_state(PlaybackState::Paused);
                }
            }
            Command::TogglePlayPause => {
                if self.state == PlaybackState::Playing {
                    self.set_state(PlaybackState::Paused);
                } else {
                    self.play();
                }
            }
            Command::Stop => {
                if self.state != PlaybackState::Stopped {
                    self.flush();
                    self.set_state(PlaybackState::Stopped);
                    self.emit(Event::Position(Duration::ZERO));
                }
            }
            Command::Next => {
                self.queue.advance(Advance::Skipped);
                self.load_current(Duration::ZERO, true);
                self.emit_queue();
            }
            Command::Previous => {
                if self.position() > RESTART_THRESHOLD || !self.queue.has_history() {
                    self.seek(Duration::ZERO);
                } else {
                    self.queue.previous();
                    self.load_current(Duration::ZERO, true);
                    self.emit_queue();
                }
            }
            Command::Seek(position) => self.seek(position),
            Command::SetVolume(volume) => {
                self.volume = volume.clamp(0.0, 1.0);
                self.feed.set_gain(gain(self.volume));
            }
            Command::SetOutputDevice(device) => {
                self.device = device;
                self.reopen();
            }
            Command::Restore(snapshot) => match Queue::from_snapshot(snapshot.queue) {
                Ok(queue) => {
                    self.queue = queue;
                    self.failures = 0;
                    if self.queue.current().is_some() {
                        self.set_state(PlaybackState::Paused);
                        self.load_current(snapshot.position, true);
                    } else {
                        self.queue_finished();
                    }
                    self.emit_queue();
                }
                Err(_) => self.emit(Event::RestoreFailed(PlaybackError::InvalidSnapshot)),
            },
        }
    }

    /// Plays the queue's current track from the start, as after a click on it.
    fn start_playing(&mut self) {
        if self.queue.current().is_none() {
            self.queue_finished();
            self.emit_queue();
            return;
        }
        if !self.connected {
            self.open_output();
        }
        self.set_state(PlaybackState::Playing);
        self.load_current(Duration::ZERO, true);
        self.emit_queue();
        if !self.connected && self.state == PlaybackState::Playing {
            self.set_state(PlaybackState::Paused);
        }
    }

    fn play(&mut self) {
        match self.state {
            PlaybackState::Playing => {}
            PlaybackState::Paused => {
                if !self.connected {
                    self.reopen();
                }
                if self.connected {
                    self.set_state(PlaybackState::Playing);
                }
            }
            PlaybackState::Stopped => {
                // After the queue ran out, Play starts the context over.
                let has_track = self.queue.current().is_some()
                    || self.queue.restart().is_some()
                    || self.queue.advance(Advance::Skipped).is_some();
                if has_track {
                    self.start_playing();
                }
            }
        }
    }

    fn seek(&mut self, position: Duration) {
        if self.queue.current().is_none() {
            return;
        }
        if self.state == PlaybackState::Stopped {
            self.set_state(PlaybackState::Paused);
        }
        self.load_current(position, false);
    }

    fn set_state(&mut self, state: PlaybackState) {
        self.feed.set_paused(state == PlaybackState::Paused);
        if state != self.state {
            self.state = state;
            self.emit(Event::StateChanged(state));
        }
    }

    fn emit(&self, event: Event) {
        // The receiver is gone only while the app shuts down.
        let _ = self.events.send(event);
    }

    fn emit_queue(&mut self) {
        let view = self.queue.view();
        self.emit(Event::QueueChanged(view));
    }

    /// Opens the requested device (or the default) with a fresh, empty ring.
    fn open_output(&mut self) {
        self.output.close();
        let paused = self.state == PlaybackState::Paused;
        let opened = self.output.open(self.device.as_deref()).and_then(|device| {
            let (feed, renderer) =
                ring::channel(device.format, RING_SECONDS, gain(self.volume), paused);
            self.output.start(renderer)?;
            Ok((device, feed))
        });
        match opened {
            Ok((device, feed)) => {
                self.install(feed, device.format, true);
                self.emit(Event::OutputChanged(OutputInfo {
                    device: device.name,
                    format: device.format,
                    fell_back: device.fell_back,
                }));
            }
            Err(error) => {
                self.output.close();
                let (feed, _) = ring::channel(NO_DEVICE, RING_SECONDS, gain(self.volume), paused);
                self.install(feed, NO_DEVICE, false);
                self.emit(Event::OutputFailed(error));
            }
        }
    }

    fn install(&mut self, feed: Feed, format: AudioFormat, connected: bool) {
        self.feed = feed;
        self.format = format;
        self.connected = connected;
        self.converter = Converter::new(format);
        self.pending.clear();
        self.pending_at = 0;
        self.backlog.clear();
        self.stream_base = 0;
        self.decoder = None;
        self.segments.clear();
        self.end_at = None;
    }

    /// Opens the output again and carries on where playback was.
    fn reopen(&mut self) {
        let position = self.position();
        self.open_output();
        if self.state != PlaybackState::Stopped && self.queue.current().is_some() {
            self.load_current(position, false);
        }
        if !self.connected && self.state == PlaybackState::Playing {
            self.set_state(PlaybackState::Paused);
        }
    }

    fn send_control(&mut self, control: Control) {
        if !self.connected {
            // Nobody reads the ring; it is replaced when a device opens.
            return;
        }
        if !self.backlog.is_empty() {
            self.backlog.push_back(control);
        } else if let Err(control) = self.feed.send(control) {
            self.backlog.push_back(control);
        }
    }

    fn send_backlog(&mut self) {
        while let Some(&control) = self.backlog.front() {
            if self.feed.send(control).is_err() {
                break;
            }
            self.backlog.pop_front();
        }
    }

    /// Drops everything decoded and tells the callback to drop what it has not
    /// played yet.
    fn flush(&mut self) {
        self.pending.clear();
        self.pending_at = 0;
        self.converter.reset();
        self.decoder = None;
        self.segments.clear();
        self.end_at = None;
        let written = self.feed.written();
        self.send_control(Control::Flush { to: written });
        self.stream_base = written;
    }

    fn channels(&self) -> usize {
        usize::from(self.format.channels)
    }

    fn pending_frames(&self) -> usize {
        (self.pending.len() - self.pending_at) / self.channels()
    }

    fn open(
        &self,
        entry: &QueueEntry,
        position: Duration,
    ) -> Result<(Box<dyn TrackStream>, Duration), PlaybackError> {
        let mut stream = self.sources.get(entry.track.source).open(&entry.track.id)?;
        let reached = if position.is_zero() {
            Duration::ZERO
        } else {
            stream.seek(position)?
        };
        Ok((stream, reached))
    }

    /// Starts the queue's current track at `position`, skipping tracks that cannot
    /// be played. `announce` sends [`Event::TrackChanged`]: a new play, not a seek.
    fn load_current(&mut self, mut position: Duration, mut announce: bool) {
        self.flush();
        loop {
            let Some(entry) = self.queue.current().cloned() else {
                self.queue_finished();
                return;
            };
            match self.open(&entry, position) {
                Ok((stream, offset)) => {
                    self.failures = 0;
                    let duration = stream.duration();
                    self.segments.push_back(Segment {
                        entry: entry.clone(),
                        start: self.stream_base,
                        offset,
                        duration,
                    });
                    self.decoder = Some(stream);
                    if announce {
                        self.emit(Event::TrackChanged(Some(NowPlaying { entry, duration })));
                    }
                    self.emit(Event::Position(offset));
                    self.last_position = Instant::now();
                    return;
                }
                Err(error) => {
                    self.emit(Event::TrackFailed {
                        track: entry.track,
                        error,
                    });
                    self.failures += 1;
                    if self.failures >= MAX_FAILURES {
                        self.queue_finished();
                        return;
                    }
                    self.queue.advance(Advance::Skipped);
                    self.emit_queue();
                    position = Duration::ZERO;
                    announce = true;
                }
            }
        }
    }

    /// Nothing more to play.
    fn queue_finished(&mut self) {
        self.flush();
        self.set_state(PlaybackState::Stopped);
        self.emit(Event::TrackChanged(None));
    }

    /// Decodes a little and moves audio towards the ring. False when there was
    /// nothing to do.
    fn fill(&mut self) -> bool {
        if self.state == PlaybackState::Stopped {
            return false;
        }
        if self.pending_at < self.pending.len() {
            let written = self.feed.write(&self.pending[self.pending_at..]);
            self.pending_at += written;
            if self.pending_at < self.pending.len() {
                return written > 0;
            }
            self.pending.clear();
            self.pending_at = 0;
        }
        let Some(stream) = self.decoder.as_mut() else {
            return false;
        };
        let result = match stream.next_chunk() {
            Ok(Some(chunk)) => self.converter.push(chunk, &mut self.pending).map(|()| true),
            Ok(None) => Ok(false),
            Err(error) => Err(error),
        };
        match result {
            Ok(true) => {}
            Ok(false) => {
                self.decoder = None;
                self.prepare_next();
            }
            Err(error) => {
                if let Some(segment) = self.segments.back() {
                    let track = segment.entry.track.clone();
                    self.emit(Event::TrackFailed { track, error });
                }
                self.decoder = None;
                self.prepare_next();
            }
        }
        true
    }

    /// Opens the track that follows the last one in the ring and starts decoding it
    /// straight after. At most one track waits ahead; with a very short track the
    /// next one is prepared once it has become audible.
    fn prepare_next(&mut self) {
        if self.segments.len() > 1 {
            return;
        }
        loop {
            let next = self.queue.peek_next().cloned();
            let Some(entry) = next else {
                self.converter.finish(&mut self.pending);
                self.end_at = Some(self.stream_base + self.converter.emitted());
                return;
            };
            match self.open(&entry, Duration::ZERO) {
                Ok((stream, _)) => {
                    self.failures = 0;
                    self.segments.push_back(Segment {
                        entry,
                        start: self.stream_base + self.converter.next_output_frame(),
                        offset: Duration::ZERO,
                        duration: stream.duration(),
                    });
                    self.decoder = Some(stream);
                    return;
                }
                Err(error) => {
                    self.emit(Event::TrackFailed {
                        track: entry.track,
                        error,
                    });
                    self.failures += 1;
                    if self.failures >= MAX_FAILURES || !self.queue.drop_next() {
                        self.converter.finish(&mut self.pending);
                        self.end_at = Some(self.stream_base + self.converter.emitted());
                        return;
                    }
                    self.emit_queue();
                }
            }
        }
    }

    /// After a queue edit: makes sure the track waiting in the ring is still the
    /// one that should follow.
    fn queue_changed(&mut self) {
        self.emit_queue();
        if self.state == PlaybackState::Stopped || self.segments.is_empty() {
            return;
        }
        let expected = self.queue.peek_next().map(|entry| entry.track.clone());
        match self.segments.get(1) {
            Some(prepared) if Some(&prepared.entry.track) == expected.as_ref() => {}
            Some(_) => {
                self.cut_prepared();
                self.prepare_next();
            }
            None if self.end_at.is_some() && expected.is_some() => {
                // The queue had run out, but now there is more: continue gaplessly.
                self.end_at = None;
                self.prepare_next();
            }
            None => {}
        }
    }

    /// Removes the prepared next track from the ring, keeping all of the current one.
    fn cut_prepared(&mut self) {
        let Some(prepared) = self.segments.pop_back() else {
            return;
        };
        self.decoder = None;
        self.end_at = None;
        // Everything up to the boundary must be out of the converter before cutting.
        self.converter.finish(&mut self.pending);
        let written = self.feed.written();
        if prepared.start >= written {
            let keep = (prepared.start - written) as usize * self.channels();
            self.pending
                .truncate((self.pending_at + keep).min(self.pending.len()));
        } else {
            self.pending.clear();
            self.pending_at = 0;
            self.send_control(Control::Cut {
                from: prepared.start,
                to: written,
            });
        }
        self.converter.reset();
        self.stream_base = self.feed.written() + self.pending_frames() as u64;
    }

    /// Follows the callback: track changes at boundaries, the end of the queue, and
    /// periodic positions.
    fn follow_playback(&mut self) {
        let read = self.feed.read();
        while self.segments.len() > 1 && read >= self.segments[1].start {
            self.segments.pop_front();
            let segment = &self.segments[0];
            let duration = segment.duration;
            let entry = self
                .queue
                .advance(Advance::Ended)
                .cloned()
                .unwrap_or_else(|| segment.entry.clone());
            self.emit(Event::TrackChanged(Some(NowPlaying { entry, duration })));
            self.emit_queue();
            self.last_position = Instant::now();
            if self.decoder.is_none() && self.end_at.is_none() {
                self.prepare_next();
            }
        }
        if let Some(end) = self.end_at
            && read >= end
            && self.segments.len() <= 1
        {
            self.queue.advance(Advance::Ended);
            self.queue_finished();
            self.emit_queue();
            return;
        }
        if self.state == PlaybackState::Playing && self.last_position.elapsed() >= POSITION_INTERVAL
        {
            self.last_position = Instant::now();
            self.emit(Event::Position(self.position()));
        }
    }

    /// Position in the audible track.
    fn position(&self) -> Duration {
        let Some(segment) = self.segments.front() else {
            return Duration::ZERO;
        };
        let frames = self.feed.read().saturating_sub(segment.start);
        let position = segment.offset
            + Duration::from_secs_f64(frames as f64 / f64::from(self.format.sample_rate));
        segment
            .duration
            .map_or(position, |duration| position.min(duration))
    }
}

#[cfg(test)]
mod tests;
