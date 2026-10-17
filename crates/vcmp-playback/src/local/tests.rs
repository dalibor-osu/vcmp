//! Decoding and gapless trimming against the fixtures in `tests/fixtures`; see
//! `make-fixtures.sh` there for how they were made.

use super::*;
use crate::test_signal::{RATE, SPLIT, TOTAL, fixture, reference};

/// Largest allowed difference from the reference sines. Lossy codecs smear the
/// edges of a separately encoded part a little; a gap or a few extra frames of
/// encoder delay would shift the phase and push errors towards the full amplitude.
fn tolerance(extension: &str) -> f32 {
    match extension {
        "wav" | "flac" => 1e-4,
        "ogg" => 0.03,
        _ => 0.1,
    }
}

const FORMATS: [&str; 4] = ["wav", "flac", "mp3", "ogg"];

fn decode(path: &Path) -> Vec<f32> {
    let mut stream = LocalStream::open(path).unwrap();
    let mut samples = Vec::new();
    while let Some(chunk) = stream.next_chunk().unwrap() {
        assert_eq!(
            chunk.format,
            AudioFormat {
                sample_rate: RATE,
                channels: 2
            }
        );
        samples.extend_from_slice(chunk.samples);
    }
    samples
}

/// Largest difference from the reference, for `samples` starting at `first_frame`.
fn max_error(samples: &[f32], first_frame: usize) -> f32 {
    samples
        .chunks(2)
        .enumerate()
        .flat_map(|(i, frame)| {
            (0..2).map(move |channel| (frame[channel] - reference(first_frame + i, channel)).abs())
        })
        .fold(0.0, f32::max)
}

#[test]
fn every_format_decodes_to_the_exact_length() {
    for extension in FORMATS {
        let a = decode(&fixture("a", extension));
        let b = decode(&fixture("b", extension));
        assert_eq!(a.len() / 2, SPLIT, "{extension}: frames in part a");
        assert_eq!(b.len() / 2, TOTAL - SPLIT, "{extension}: frames in part b");
    }
}

#[test]
fn consecutive_parts_join_without_a_gap() {
    for extension in FORMATS {
        let mut joined = decode(&fixture("a", extension));
        joined.extend(decode(&fixture("b", extension)));
        let error = max_error(&joined, 0);
        assert!(error < tolerance(extension), "{extension}: error {error}");
    }
}

#[test]
fn durations_exclude_delay_and_padding() {
    for extension in FORMATS {
        let stream = LocalStream::open(&fixture("a", extension)).unwrap();
        let expected = SPLIT as f64 / f64::from(RATE);
        let duration = stream.duration().unwrap().as_secs_f64();
        assert!(
            (duration - expected).abs() < 1e-6,
            "{extension}: {duration}"
        );
    }
}

#[test]
fn seeking_lands_on_the_exact_sample() {
    for extension in FORMATS {
        for target_frame in [0, 1, 11_025, 20_000] {
            let mut stream = LocalStream::open(&fixture("a", extension)).unwrap();
            let target = Duration::from_secs_f64(target_frame as f64 / f64::from(RATE));
            let reached = stream.seek(target).unwrap();
            assert!(
                (reached.as_secs_f64() - target.as_secs_f64()).abs() < 1e-4,
                "{extension}: reached {reached:?} for {target:?}"
            );
            let mut rest = Vec::new();
            while let Some(chunk) = stream.next_chunk().unwrap() {
                rest.extend_from_slice(chunk.samples);
            }
            assert_eq!(
                rest.len() / 2,
                SPLIT - target_frame,
                "{extension} at {target_frame}"
            );
            let error = max_error(&rest, target_frame);
            assert!(
                error < tolerance(extension),
                "{extension} at {target_frame}: {error}"
            );
        }
    }
}

#[test]
fn seeking_back_after_the_end_plays_again() {
    let mut stream = LocalStream::open(&fixture("a", "flac")).unwrap();
    while stream.next_chunk().unwrap().is_some() {}
    stream.seek(Duration::ZERO).unwrap();
    assert!(stream.next_chunk().unwrap().is_some());
}

#[test]
fn seeking_past_the_end_ends_the_stream() {
    for extension in FORMATS {
        let mut stream = LocalStream::open(&fixture("a", extension)).unwrap();
        stream.seek(Duration::from_secs(5)).unwrap();
        let mut left = 0;
        while let Some(chunk) = stream.next_chunk().unwrap() {
            left += chunk.samples.len();
        }
        assert_eq!(left, 0, "{extension}");
    }
}

#[test]
fn missing_files_and_other_files_are_reported() {
    let missing = LocalSource
        .open("/nonexistent/vcmp/track.flac")
        .err()
        .unwrap();
    assert!(
        matches!(missing, PlaybackError::Io(io::ErrorKind::NotFound, _)),
        "{missing:?}"
    );
    let script = fixture("a", "flac").with_file_name("make-fixtures.sh");
    let not_audio = LocalSource.open(script.to_str().unwrap()).err().unwrap();
    assert!(
        matches!(not_audio, PlaybackError::Unsupported(_)),
        "{not_audio:?}"
    );
}
