use std::f64::consts::TAU;

use super::*;

fn format(sample_rate: u32, channels: u16) -> AudioFormat {
    AudioFormat {
        sample_rate,
        channels,
    }
}

fn sine(rate: u32, frequency: f64, frames: std::ops::Range<usize>) -> Vec<f32> {
    frames
        .map(|i| (0.5 * (TAU * frequency * i as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}

/// Pushes `samples` in uneven chunk sizes, as a decoder would.
fn push_all(converter: &mut Converter, input: AudioFormat, samples: &[f32], out: &mut Vec<f32>) {
    let channels = usize::from(input.channels);
    let mut rng = fastrand::Rng::with_seed(4);
    let mut rest = samples;
    while !rest.is_empty() {
        let frames = rng.usize(1..3000).min(rest.len() / channels);
        let (chunk, tail) = rest.split_at(frames * channels);
        converter
            .push(
                AudioChunk {
                    format: input,
                    samples: chunk,
                },
                out,
            )
            .unwrap();
        rest = tail;
    }
}

#[test]
fn same_format_passes_through_unchanged() {
    let mut converter = Converter::new(format(44_100, 2));
    let input: Vec<f32> = (0..2000).map(|i| i as f32 / 2000.0).collect();
    let mut out = Vec::new();
    push_all(&mut converter, format(44_100, 2), &input, &mut out);
    assert_eq!(converter.next_output_frame(), 1000);
    converter.finish(&mut out);
    assert_eq!(out, input);
    assert_eq!(converter.emitted(), 1000);
}

#[test]
fn resampling_yields_exactly_the_rounded_length() {
    for (from, to) in [
        (44_100, 48_000),
        (48_000, 44_100),
        (22_050, 48_000),
        (96_000, 44_100),
        (8_000, 44_100),
    ] {
        for frames in [0, 1, 100, 1029, 22_433, 44_100] {
            let mut converter = Converter::new(format(to, 1));
            let mut out = Vec::new();
            push_all(
                &mut converter,
                format(from, 1),
                &sine(from, 440.0, 0..frames),
                &mut out,
            );
            converter.finish(&mut out);
            let expected = (frames as u64 * u64::from(to) + u64::from(from) / 2) / u64::from(from);
            assert_eq!(
                out.len() as u64,
                expected,
                "{from} -> {to}, {frames} frames"
            );
            assert_eq!(converter.emitted(), expected);
        }
    }
}

#[test]
fn resampling_keeps_the_signal_and_its_timing() {
    for (from, to) in [(44_100, 48_000), (48_000, 44_100), (88_200, 48_000)] {
        let mut converter = Converter::new(format(to, 1));
        let mut out = Vec::new();
        push_all(
            &mut converter,
            format(from, 1),
            &sine(from, 1000.0, 0..from as usize),
            &mut out,
        );
        converter.finish(&mut out);
        // rubato's FFT resampler adds a constant latency of half an input sample;
        // allow a shift below one output sample. The very edges ring because the
        // tone starts and stops abruptly.
        let middle = 300..out.len() - 300;
        let (shift, error) = (-100..=100)
            .map(|k| {
                let shift = f64::from(k) / 100.0;
                let error = middle
                    .clone()
                    .map(|i| {
                        let ideal = 0.5 * (TAU * 1000.0 * (i as f64 + shift) / f64::from(to)).sin();
                        (out[i] - ideal as f32).abs()
                    })
                    .fold(0.0, f32::max);
                (shift, error)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(
            error < 1e-3,
            "{from} -> {to}: error {error} at shift {shift}"
        );
    }
}

#[test]
fn tracks_in_one_run_resample_as_one_signal() {
    let signal = sine(44_100, 440.0, 0..44_100);
    let (a, b) = signal.split_at(22_433);
    let mut whole = Vec::new();
    let mut converter = Converter::new(format(48_000, 1));
    push_all(&mut converter, format(44_100, 1), &signal, &mut whole);
    converter.finish(&mut whole);

    let mut joined = Vec::new();
    let mut converter = Converter::new(format(48_000, 1));
    push_all(&mut converter, format(44_100, 1), a, &mut joined);
    let boundary = converter.next_output_frame();
    assert_eq!(boundary, (22_433 * 48_000 + 22_050) / 44_100);
    push_all(&mut converter, format(44_100, 1), b, &mut joined);
    converter.finish(&mut joined);
    assert_eq!(
        joined, whole,
        "splitting the input must not change the output"
    );
}

#[test]
fn a_format_change_ends_the_run_exactly_at_the_boundary() {
    let mut converter = Converter::new(format(48_000, 1));
    let mut out = Vec::new();
    push_all(
        &mut converter,
        format(44_100, 1),
        &sine(44_100, 440.0, 0..10_000),
        &mut out,
    );
    let boundary = converter.next_output_frame();
    let second = vec![0.25; 500];
    push_all(&mut converter, format(48_000, 1), &second, &mut out);
    assert_eq!(out.len() as u64, boundary + 500);
    assert_eq!(&out[boundary as usize..], &second[..]);
    assert_eq!(converter.next_output_frame(), boundary + 500);
}

#[test]
fn reset_starts_counting_again() {
    let mut converter = Converter::new(format(48_000, 2));
    let mut out = Vec::new();
    push_all(
        &mut converter,
        format(44_100, 2),
        &vec![0.1; 4000],
        &mut out,
    );
    converter.reset();
    assert_eq!(converter.emitted(), 0);
    assert_eq!(converter.next_output_frame(), 0);
}

fn mapped(input: &[f32], from: u16, to: u16) -> Vec<f32> {
    let mut converter = Converter::new(format(44_100, to));
    let mut out = Vec::new();
    converter
        .push(
            AudioChunk {
                format: format(44_100, from),
                samples: input,
            },
            &mut out,
        )
        .unwrap();
    out
}

#[test]
fn mono_plays_on_both_speakers() {
    assert_eq!(mapped(&[0.5, -0.25], 1, 2), [0.5, 0.5, -0.25, -0.25]);
    assert_eq!(mapped(&[0.5], 1, 4), [0.5, 0.5, 0.0, 0.0]);
}

#[test]
fn stereo_to_mono_averages() {
    assert_eq!(mapped(&[0.5, 0.25, -1.0, 1.0], 2, 1), [0.375, 0.0]);
}

#[test]
fn stereo_on_surround_uses_the_front_pair() {
    assert_eq!(mapped(&[0.5, -0.5], 2, 6), [0.5, -0.5, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn surround_downmixes_to_stereo_without_clipping() {
    // 5.1: FL FR FC LFE BL BR
    let left_only = mapped(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 6, 2);
    let centre = mapped(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 6, 2);
    let lfe = mapped(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0], 6, 2);
    let back_right = mapped(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0], 6, 2);
    let full = mapped(&[1.0; 6], 6, 2);
    assert!(left_only[0] > 0.0 && left_only[1] == 0.0);
    assert!(centre[0] > 0.0 && centre[0] == centre[1]);
    assert_eq!(lfe, [0.0, 0.0]);
    assert!(back_right[0] == 0.0 && back_right[1] > 0.0);
    assert!(full.iter().all(|&s| s <= 1.0 + 1e-6), "{full:?}");
    for channels in 3..=8 {
        let full = mapped(&vec![1.0; channels], channels as u16, 2);
        assert!(
            full.iter().all(|&s| (s - 1.0).abs() < 1e-6),
            "{channels}: {full:?}"
        );
    }
}

#[test]
fn surround_on_another_surround_keeps_the_shared_channels() {
    assert_eq!(
        mapped(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, 8),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0]
    );
    assert_eq!(
        mapped(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], 8, 6),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
}

#[test]
fn unknown_layouts_use_their_first_two_channels() {
    let input: Vec<f32> = (1..=10).map(|i| i as f32).collect();
    assert_eq!(mapped(&input, 10, 2), [1.0, 2.0]);
}
