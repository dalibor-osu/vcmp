//! Converts decoded audio to the output format: channels first, then sample rate.
//!
//! Consecutive tracks with the same format form one run: they go through the same
//! resampler without a break, so a gapless album at 44.1 kHz on a 48 kHz device has
//! no seam where the tracks meet. A format change ends the run: the resampler tail is
//! flushed (exactly `round(input frames * ratio)` frames come out in total) and a new
//! run starts. The engine uses [`Converter::next_output_frame`] to know where in the
//! output the next track begins.

use std::f32::consts::FRAC_1_SQRT_2;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

use crate::error::PlaybackError;
use crate::source::{AudioChunk, AudioFormat};

pub(crate) struct Converter {
    output: AudioFormat,
    /// Input format of the current run, `None` between runs.
    input: Option<AudioFormat>,
    mixed: Vec<f32>,
    resampler: Option<Resampler>,
    /// Output frames emitted since the last reset.
    emitted: u64,
    /// `emitted` when the current run started.
    run_start: u64,
    /// Input frames pushed into the current run.
    run_input: u64,
}

impl Converter {
    pub(crate) fn new(output: AudioFormat) -> Self {
        Converter {
            output,
            input: None,
            mixed: Vec::new(),
            resampler: None,
            emitted: 0,
            run_start: 0,
            run_input: 0,
        }
    }

    /// Converts `chunk` and appends the result to `out`. A resampler holds back a
    /// little audio until more input or [`finish`](Self::finish) pushes it out.
    pub(crate) fn push(
        &mut self,
        chunk: AudioChunk<'_>,
        out: &mut Vec<f32>,
    ) -> Result<(), PlaybackError> {
        if self.input != Some(chunk.format) {
            self.finish(out);
            self.start_run(chunk.format)?;
        }
        let from = usize::from(chunk.format.channels);
        let to = usize::from(self.output.channels);
        let frames = chunk.samples.len() / from;
        let samples = &chunk.samples[..frames * from];
        self.run_input += frames as u64;
        let mapped: &[f32] = if from == to {
            samples
        } else {
            map_channels(samples, from, to, &mut self.mixed);
            &self.mixed
        };
        match &mut self.resampler {
            Some(resampler) => self.emitted += resampler.push(mapped, out),
            None => {
                out.extend_from_slice(mapped);
                self.emitted += frames as u64;
            }
        }
        Ok(())
    }

    /// Ends the current run: whatever the resampler still holds goes to `out`.
    pub(crate) fn finish(&mut self, out: &mut Vec<f32>) {
        let target = self.next_output_frame();
        if let Some(resampler) = &mut self.resampler {
            self.emitted += resampler.finish(target - self.emitted, out);
        }
        self.input = None;
        self.resampler = None;
        self.run_start = self.emitted;
        self.run_input = 0;
    }

    /// Drops everything, as when playback jumps elsewhere.
    pub(crate) fn reset(&mut self) {
        self.input = None;
        self.resampler = None;
        self.emitted = 0;
        self.run_start = 0;
        self.run_input = 0;
    }

    /// The output frame (counted since the last reset) at which the next input
    /// frame will come out.
    pub(crate) fn next_output_frame(&self) -> u64 {
        self.run_start + self.run_output(self.run_input)
    }

    /// Output frames emitted since the last reset.
    pub(crate) fn emitted(&self) -> u64 {
        self.emitted
    }

    fn run_output(&self, input_frames: u64) -> u64 {
        match self.input {
            Some(input) if input.sample_rate != self.output.sample_rate => {
                let (from, to) = (
                    u64::from(input.sample_rate),
                    u64::from(self.output.sample_rate),
                );
                (input_frames * to + from / 2) / from
            }
            _ => input_frames,
        }
    }

    fn start_run(&mut self, input: AudioFormat) -> Result<(), PlaybackError> {
        self.resampler = if input.sample_rate == self.output.sample_rate {
            None
        } else {
            Some(Resampler::new(
                input.sample_rate,
                self.output.sample_rate,
                self.output.channels,
            )?)
        };
        self.input = Some(input);
        self.run_start = self.emitted;
        self.run_input = 0;
        Ok(())
    }
}

/// Input frames per resampler call. Bigger blocks give a slightly higher cutoff.
const RESAMPLER_CHUNK: usize = 1024;

/// rubato's FFT resampler fed with arbitrary amounts of interleaved audio.
struct Resampler {
    inner: Fft<f32>,
    channels: usize,
    input: Vec<f32>,
    filled: usize,
    output: Vec<f32>,
    /// Leading output frames still to drop: the filter's delay.
    delay: usize,
}

impl Resampler {
    fn new(from: u32, to: u32, channels: u16) -> Result<Self, PlaybackError> {
        let channels = usize::from(channels);
        let inner = Fft::<f32>::new(
            from as usize,
            to as usize,
            RESAMPLER_CHUNK,
            channels,
            FixedSync::Both,
        )
        .map_err(|error| {
            PlaybackError::Unsupported(format!("resampling {from} Hz to {to} Hz: {error}"))
        })?;
        Ok(Resampler {
            input: vec![0.0; inner.input_frames_max() * channels],
            output: vec![0.0; inner.output_frames_max() * channels],
            delay: inner.output_delay(),
            filled: 0,
            channels,
            inner,
        })
    }

    /// Returns the number of frames appended to `out`.
    fn push(&mut self, mut samples: &[f32], out: &mut Vec<f32>) -> u64 {
        let channels = self.channels;
        let chunk = self.inner.input_frames_next();
        let mut emitted = 0;
        while !samples.is_empty() {
            let take = (chunk - self.filled).min(samples.len() / channels);
            self.input[self.filled * channels..(self.filled + take) * channels]
                .copy_from_slice(&samples[..take * channels]);
            self.filled += take;
            samples = &samples[take * channels..];
            if self.filled == chunk {
                emitted += self.process(None, u64::MAX, out);
                self.filled = 0;
            }
        }
        emitted
    }

    /// Pushes out the rest, stopping after `remaining` more frames.
    fn finish(&mut self, mut remaining: u64, out: &mut Vec<f32>) -> u64 {
        let mut emitted = 0;
        let mut partial = self.filled;
        // Every call yields a full output chunk, so this ends after a few rounds; the
        // bound only guards against a resampler that stops producing.
        for _ in 0..remaining + 4 {
            if remaining == 0 {
                break;
            }
            let produced = self.process(Some(partial), remaining, out);
            remaining -= produced;
            emitted += produced;
            partial = 0;
        }
        self.filled = 0;
        emitted
    }

    fn process(&mut self, partial: Option<usize>, limit: u64, out: &mut Vec<f32>) -> u64 {
        let channels = self.channels;
        let frames_in = self.inner.input_frames_next();
        let input = InterleavedSlice::new(&self.input[..], channels, frames_in)
            .expect("input buffer holds a full chunk");
        let capacity = self.output.len() / channels;
        let mut output = InterleavedSlice::new_mut(&mut self.output[..], channels, capacity)
            .expect("output buffer holds the largest chunk");
        let indexing = partial.map(|frames| Indexing::new().partial_len(frames));
        let (_, produced) = self
            .inner
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .expect("buffers are sized by the resampler");
        let start = self.delay.min(produced);
        self.delay -= start;
        let end = start + (produced - start).min(usize::try_from(limit).unwrap_or(usize::MAX));
        out.extend_from_slice(&self.output[start * channels..end * channels]);
        (end - start) as u64
    }
}

/// Weights of each input channel in the left and right output, for the default
/// channel orders of WAVE and FLAC; LFE is left out. `None` for layouts without a
/// known order, which then use their first two channels.
fn downmix_weights(channels: usize) -> Option<[(f32, f32); 8]> {
    const C: f32 = FRAC_1_SQRT_2;
    const L: (f32, f32) = (1.0, 0.0);
    const R: (f32, f32) = (0.0, 1.0);
    const CENTRE: (f32, f32) = (C, C);
    const SL: (f32, f32) = (C, 0.0);
    const SR: (f32, f32) = (0.0, C);
    const BC: (f32, f32) = (0.5, 0.5);
    const NONE: (f32, f32) = (0.0, 0.0);
    let mut weights = match channels {
        3 => [L, R, CENTRE, NONE, NONE, NONE, NONE, NONE],
        4 => [L, R, SL, SR, NONE, NONE, NONE, NONE],
        5 => [L, R, CENTRE, SL, SR, NONE, NONE, NONE],
        6 => [L, R, CENTRE, NONE, SL, SR, NONE, NONE],
        7 => [L, R, CENTRE, NONE, BC, SL, SR, NONE],
        8 => [L, R, CENTRE, NONE, SL, SR, SL, SR],
        _ => return None,
    };
    // Scale so that a full-scale signal in every channel cannot clip.
    let total: f32 = weights.iter().map(|w| w.0).sum();
    for weight in &mut weights {
        weight.0 /= total;
        weight.1 /= total;
    }
    Some(weights)
}

/// Maps interleaved frames from `from` channels to `to` channels into `out`.
fn map_channels(input: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(input.len() / from * to);
    let weights = downmix_weights(from);
    for frame in input.chunks_exact(from) {
        if from > 2 && to > 2 {
            // Surround to surround: the standard orders share their prefix.
            let common = from.min(to);
            out.extend_from_slice(&frame[..common]);
            out.extend(std::iter::repeat_n(0.0, to - common));
            continue;
        }
        let (left, right) = match (from, &weights) {
            (1, _) => (frame[0], frame[0]),
            (_, Some(weights)) => frame
                .iter()
                .zip(weights)
                .fold((0.0, 0.0), |(l, r), (s, w)| (l + s * w.0, r + s * w.1)),
            _ => (frame[0], frame[1]),
        };
        if to == 1 {
            out.push(0.5 * (left + right));
        } else {
            out.push(left);
            out.push(right);
            out.extend(std::iter::repeat_n(0.0, to - 2));
        }
    }
}

#[cfg(test)]
mod tests;
