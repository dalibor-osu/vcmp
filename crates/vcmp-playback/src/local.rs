//! Local files, decoded with symphonia.

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::{Error, SeekErrorKind};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

use crate::error::PlaybackError;
use crate::source::{AudioChunk, AudioFormat, TrackSource, TrackStream};

/// Plays files from disk. The track id is the file path.
///
/// Formats: MP3, FLAC, WAV and Ogg Vorbis. Gapless trimming comes from symphonia:
/// MP3 needs a LAME/Xing (or Lavf/Lavc) header with delay and padding, iTunes'
/// iTunSMPB is not read; Vorbis uses the Ogg granule positions; FLAC and WAV have
/// nothing to trim.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalSource;

impl TrackSource for LocalSource {
    fn open(&self, id: &str) -> Result<Box<dyn TrackStream>, PlaybackError> {
        Ok(Box::new(LocalStream::open(Path::new(id))?))
    }
}

/// Damaged packets skipped in a row before the file counts as broken.
const MAX_BAD_PACKETS: usize = 64;
/// Timestamps decoded and dropped before a seek target (about 0.1 s at 44.1 kHz).
const SEEK_PREROLL: i64 = 4096;

struct LocalStream {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    duration: Option<Duration>,
    /// After a seek: output before this timestamp is dropped.
    skip_until: Option<Timestamp>,
    ended: bool,
    samples: Vec<f32>,
}

impl LocalStream {
    fn open(path: &Path) -> Result<LocalStream, PlaybackError> {
        let file = File::open(path)?;
        let source = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let reader = symphonia::default::get_probe().probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| PlaybackError::Unsupported("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| PlaybackError::Unsupported("no audio track".into()))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default().gapless(true))?;
        // Playable frames exclude delay and padding, so prefer them to the raw length.
        let duration = match (track.num_frames, params.sample_rate) {
            (Some(frames), Some(rate)) if rate > 0 => {
                Some(Duration::from_secs_f64(frames as f64 / f64::from(rate)))
            }
            _ => track
                .time_base
                .zip(track.duration)
                .and_then(|(base, length)| base.calc_duration(length))
                .map(|time| Duration::from_secs_f64(time.as_secs_f64().max(0.0))),
        };
        let track_id = track.id;
        let time_base = track.time_base;
        Ok(LocalStream {
            reader,
            decoder,
            track_id,
            time_base,
            duration,
            skip_until: None,
            ended: false,
            samples: Vec::new(),
        })
    }
}

fn is_end_of_stream(error: &Error) -> bool {
    match error {
        Error::IoError(error) => error.kind() == io::ErrorKind::UnexpectedEof,
        // The track list changed mid-file (a chained Ogg stream): play what came
        // before as the whole track.
        Error::ResetRequired => true,
        _ => false,
    }
}

impl TrackStream for LocalStream {
    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn next_chunk(&mut self) -> Result<Option<AudioChunk<'_>>, PlaybackError> {
        let mut bad_packets = 0;
        while !self.ended {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(error) if is_end_of_stream(&error) => break,
                Err(error) => return Err(error.into()),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(Error::DecodeError(_) | Error::IoError(_)) if bad_packets < MAX_BAD_PACKETS => {
                    bad_packets += 1;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            bad_packets = 0;
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let mut skip = 0;
            if let Some(target) = self.skip_until {
                // With gapless trimming on, the first decoded frame is at pts + trim_start.
                let first = packet.pts.saturating_add(packet.trim_start);
                let behind = target.get().saturating_sub(first.get());
                if behind >= frames as i64 {
                    continue;
                }
                skip = behind.max(0) as usize;
                self.skip_until = None;
            }
            let format = AudioFormat {
                sample_rate: decoded.spec().rate(),
                channels: decoded.spec().channels().count() as u16,
            };
            if format.channels == 0 || format.sample_rate == 0 {
                return Err(PlaybackError::Unsupported(
                    "no channels or sample rate".into(),
                ));
            }
            decoded
                .slice(skip..)
                .copy_to_vec_interleaved(&mut self.samples);
            return Ok(Some(AudioChunk {
                format,
                samples: &self.samples,
            }));
        }
        self.ended = true;
        Ok(None)
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, PlaybackError> {
        let Some(base) = self.time_base else {
            return self.seek_to_time(position);
        };
        // Converting through `Time` truncates to whole nanoseconds, which can land one
        // sample early, so round to the time base directly.
        let (numer, denom) = (u128::from(base.numer.get()), u128::from(base.denom.get()));
        let scale = numer * 1_000_000_000;
        let ts =
            i64::try_from((position.as_nanos() * denom + scale / 2) / scale).unwrap_or(i64::MAX);
        let target = Timestamp::new(ts);
        // Start decoding a little earlier: right after a reset, decoders such as MP3
        // lack the overlap from the previous frame, and that part is thrown away.
        let preroll = Timestamp::new(ts.saturating_sub(SEEK_PREROLL).max(0));
        let to = SeekTo::Timestamp {
            ts: preroll,
            track_id: self.track_id,
        };
        match self.reader.seek(SeekMode::Accurate, to) {
            Ok(_) => {
                self.decoder.reset();
                self.ended = false;
                self.skip_until = Some(target);
                Ok(base.calc_time(target).map_or(position, |time| {
                    Duration::from_secs_f64(time.as_secs_f64().max(0.0))
                }))
            }
            Err(Error::SeekError(SeekErrorKind::OutOfRange)) => self.seek_past_end(position),
            Err(error) => Err(error.into()),
        }
    }
}

impl LocalStream {
    fn seek_to_time(&mut self, position: Duration) -> Result<Duration, PlaybackError> {
        let time = Time::try_from_secs_f64(position.as_secs_f64())
            .ok_or_else(|| PlaybackError::Decode("seek position out of range".into()))?;
        let to = SeekTo::Time {
            time,
            track_id: Some(self.track_id),
        };
        match self.reader.seek(SeekMode::Accurate, to) {
            Ok(seeked) => {
                self.decoder.reset();
                self.ended = false;
                self.skip_until = Some(seeked.required_ts);
                Ok(position)
            }
            Err(Error::SeekError(SeekErrorKind::OutOfRange)) => self.seek_past_end(position),
            Err(error) => Err(error.into()),
        }
    }

    fn seek_past_end(&mut self, position: Duration) -> Result<Duration, PlaybackError> {
        self.ended = true;
        Ok(self.duration.map_or(position, |d| d.min(position)))
    }
}

#[cfg(test)]
mod tests;
