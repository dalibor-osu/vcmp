//! The hand-over between the decoder thread and the audio callback.
//!
//! Decoded, converted frames go through a lock-free single-producer ring buffer
//! (rtrb). Frames are numbered by how many were ever written: frame `n` is the `n`th
//! frame the engine wrote since the output opened. Track boundaries, seeks and skips
//! are all expressed in that numbering, so the callback needs no knowledge of tracks.
//!
//! The callback side, [`Renderer`], must never allocate, lock, wait or do I/O. It
//! only touches the two rtrb queues (wait-free) and a few atomics. A test with a
//! counting allocator (`no_allocation_in_the_callback`) holds it to that.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cpal::{FromSample, Sample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::source::AudioFormat;

/// State shared by the engine and the callback.
#[derive(Debug)]
pub(crate) struct Shared {
    /// Frames the callback has taken from the ring, played or skipped.
    read: AtomicU64,
    paused: AtomicBool,
    /// Output amplitude, as `f32` bits.
    gain: AtomicU32,
}

/// Instructions for the callback, sent after the frames they refer to were written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    /// Fade out now, then drop everything before frame `to`.
    Flush { to: u64 },
    /// On reaching frame `from`, continue at frame `to`: the track that was to follow
    /// has been replaced.
    Cut { from: u64, to: u64 },
}

/// Time for a full-scale gain change: pause, resume, seek and volume changes fade
/// instead of clicking.
const RAMP_SECONDS: f32 = 0.01;
/// Controls that can wait for the callback.
const CONTROL_CAPACITY: usize = 64;

/// The engine's side.
pub(crate) struct Feed {
    samples: Producer<f32>,
    control: Producer<Control>,
    shared: Arc<Shared>,
    channels: usize,
    written: u64,
}

/// The audio callback's side.
pub struct Renderer {
    samples: Consumer<f32>,
    control: Consumer<Control>,
    shared: Arc<Shared>,
    channels: usize,
    read: u64,
    gain: f32,
    step: f32,
    flush_to: Option<u64>,
    cut: Option<(u64, u64)>,
}

/// A ring holding `seconds` of audio in `format`.
pub(crate) fn channel(
    format: AudioFormat,
    seconds: f32,
    gain: f32,
    paused: bool,
) -> (Feed, Renderer) {
    let channels = usize::from(format.channels.max(1));
    let frames = (format.sample_rate as f32 * seconds) as usize;
    let (sample_producer, sample_consumer) = RingBuffer::new(frames.max(1) * channels);
    let (control_producer, control_consumer) = RingBuffer::new(CONTROL_CAPACITY);
    let shared = Arc::new(Shared {
        read: AtomicU64::new(0),
        paused: AtomicBool::new(paused),
        gain: AtomicU32::new(gain.to_bits()),
    });
    let feed = Feed {
        samples: sample_producer,
        control: control_producer,
        shared: shared.clone(),
        channels,
        written: 0,
    };
    let renderer = Renderer {
        samples: sample_consumer,
        control: control_consumer,
        shared,
        channels,
        read: 0,
        gain: 0.0,
        step: 1.0 / (format.sample_rate.max(1) as f32 * RAMP_SECONDS),
        flush_to: None,
        cut: None,
    };
    (feed, renderer)
}

impl Feed {
    /// Writes as many whole frames of `samples` as fit; returns the samples written.
    pub(crate) fn write(&mut self, samples: &[f32]) -> usize {
        let room = self.samples.slots() / self.channels * self.channels;
        let count = room.min(samples.len() / self.channels * self.channels);
        if count > 0 {
            self.samples
                .push_entire_slice(&samples[..count])
                .expect("checked free slots");
            self.written += (count / self.channels) as u64;
        }
        count
    }

    /// Frames written so far.
    pub(crate) fn written(&self) -> u64 {
        self.written
    }

    /// Frames the callback has taken so far.
    pub(crate) fn read(&self) -> u64 {
        self.shared.read.load(Ordering::Acquire)
    }

    /// Hands a control to the callback; gives it back when the queue is full.
    pub(crate) fn send(&mut self, control: Control) -> Result<(), Control> {
        self.control
            .push(control)
            .map_err(|rtrb::PushError::Full(control)| control)
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.shared.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
}

impl Renderer {
    /// Fills `out` (interleaved, in the output's channel count) with the next frames,
    /// or silence where there are none. This is the body of the audio callback.
    pub fn render<T: Sample + FromSample<f32>>(&mut self, out: &mut [T]) {
        while let Ok(control) = self.control.pop() {
            match control {
                Control::Flush { to } => {
                    self.flush_to = Some(self.flush_to.map_or(to, |old| old.max(to)));
                    if self.cut.is_some_and(|(_, cut_to)| cut_to <= to) {
                        self.cut = None;
                    }
                }
                Control::Cut { from, to } => {
                    self.cut = Some(match self.cut {
                        Some((old_from, old_to)) => (old_from.min(from), old_to.max(to)),
                        None => (from, to),
                    });
                }
            }
        }
        let channels = self.channels;
        let paused = self.shared.paused.load(Ordering::Relaxed);
        let volume = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));
        let total = out.len() / channels;
        let mut done = 0;
        while done < total {
            let flushing = self.flush_to.is_some();
            let stopping = flushing || paused;
            let target = if stopping { 0.0 } else { volume };
            if let Some(to) = self.flush_to
                && self.gain == 0.0
            {
                if !self.skip_to(to) {
                    break;
                }
                self.flush_to = None;
                continue;
            }
            if let Some((from, to)) = self.cut
                && self.read >= from
            {
                if !self.skip_to(to) {
                    break;
                }
                self.cut = None;
                continue;
            }
            if paused && self.gain == 0.0 {
                break;
            }
            let mut frames = (total - done).min(self.samples.slots() / channels);
            if let Some((from, _)) = self.cut {
                frames = frames.min((from - self.read) as usize);
            }
            if let Some(to) = self.flush_to {
                frames = frames.min(to.saturating_sub(self.read) as usize);
            }
            if stopping {
                // Fading out: stop reading once silent, so a pause keeps its place.
                frames = frames.min((self.gain / self.step).ceil() as usize);
            }
            if frames == 0 {
                if flushing {
                    // The old audio ran out before the fade finished.
                    self.gain = 0.0;
                    continue;
                }
                break;
            }
            let Ok(chunk) = self.samples.read_chunk(frames * channels) else {
                break;
            };
            let (first, second) = chunk.as_slices();
            let destination = &mut out[done * channels..(done + frames) * channels];
            for (frame, source) in destination.chunks_exact_mut(channels).zip(
                first
                    .chunks_exact(channels)
                    .chain(second.chunks_exact(channels)),
            ) {
                self.gain += (target - self.gain).clamp(-self.step, self.step);
                for (sample, &value) in frame.iter_mut().zip(source) {
                    *sample = T::from_sample(value * self.gain);
                }
            }
            chunk.commit_all();
            self.read += frames as u64;
            done += frames;
        }
        out[done * channels..].fill(T::EQUILIBRIUM);
        self.shared.read.store(self.read, Ordering::Release);
    }

    /// Drops frames up to frame `to`. False if they have not all arrived yet, which
    /// the ordering of writes before controls rules out; the skip is retried then.
    fn skip_to(&mut self, to: u64) -> bool {
        let wanted = to.saturating_sub(self.read) as usize * self.channels;
        let available = self.samples.slots() / self.channels * self.channels;
        let count = wanted.min(available);
        if let Ok(chunk) = self.samples.read_chunk(count) {
            chunk.commit_all();
            self.read += (count / self.channels) as u64;
        }
        self.read >= to
    }

    /// Frames waiting in the ring.
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.samples.slots() / self.channels
    }

    /// Frames taken from the ring so far, played or skipped.
    #[cfg(test)]
    pub(crate) fn taken(&self) -> u64 {
        self.read
    }
}

#[cfg(test)]
mod tests;
