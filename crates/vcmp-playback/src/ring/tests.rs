use super::*;
use crate::alloc_counter::allocations_in;

/// At 100 Hz a gain ramp takes a single frame, so values come out exact.
const INSTANT: AudioFormat = AudioFormat {
    sample_rate: 100,
    channels: 2,
};
const REAL: AudioFormat = AudioFormat {
    sample_rate: 48_000,
    channels: 2,
};

fn frames(values: impl IntoIterator<Item = f32>) -> Vec<f32> {
    values.into_iter().flat_map(|v| [v, -v]).collect()
}

fn render(renderer: &mut Renderer, count: usize) -> Vec<f32> {
    let mut out = vec![9.0; count * 2];
    renderer.render(&mut out);
    out
}

fn left(samples: &[f32]) -> Vec<f32> {
    samples.chunks(2).map(|f| f[0]).collect()
}

#[test]
fn frames_come_out_in_order() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 1.0, false);
    let input = frames((1..=50).map(|i| i as f32 / 100.0));
    assert_eq!(feed.write(&input), 100);
    assert_eq!(render(&mut renderer, 50), input);
    assert_eq!(feed.read(), 50);
}

#[test]
fn only_whole_frames_are_written() {
    let (mut feed, _renderer) = channel(INSTANT, 0.03, 1.0, false);
    assert_eq!(feed.write(&[0.1; 7]), 6, "three frames fit");
    assert_eq!(feed.written(), 3);
    assert_eq!(feed.write(&[0.1; 2]), 0);
}

#[test]
fn an_underrun_plays_silence_and_keeps_the_place() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 1.0, false);
    feed.write(&frames([0.5; 3]));
    let out = render(&mut renderer, 5);
    assert_eq!(left(&out), [0.5, 0.5, 0.5, 0.0, 0.0]);
    assert_eq!(feed.read(), 3);
    feed.write(&frames([0.25]));
    assert_eq!(left(&render(&mut renderer, 1)), [0.25]);
}

#[test]
fn pausing_fades_out_and_resumes_where_it_stopped() {
    let (mut feed, mut renderer) = channel(REAL, 1.0, 1.0, false);
    feed.write(&frames([0.5; 4800]));
    render(&mut renderer, 1000);
    feed.set_paused(true);
    let out = left(&render(&mut renderer, 1000));
    let fade = (1.0 / renderer.step) as usize;
    assert!(out[..fade].windows(2).all(|w| w[1] <= w[0]), "fades out");
    assert!(
        out[fade + 1..].iter().all(|&s| s == 0.0),
        "silent once faded"
    );
    let stopped_at = feed.read();
    render(&mut renderer, 1000);
    assert_eq!(feed.read(), stopped_at, "a pause consumes nothing");
    feed.set_paused(false);
    let out = left(&render(&mut renderer, 1000));
    assert!(
        out[..fade].windows(2).all(|w| w[1] >= w[0]),
        "fades back in"
    );
    assert_eq!(out[fade + 1], 0.5);
}

#[test]
fn volume_scales_and_changes_smoothly() {
    let (mut feed, mut renderer) = channel(REAL, 1.0, 0.25, false);
    feed.write(&frames([1.0; 4800]));
    let out = left(&render(&mut renderer, 1000));
    assert_eq!(out[999], 0.25);
    feed.set_gain(1.0);
    let out = left(&render(&mut renderer, 1000));
    assert!(
        out.windows(2).all(|w| w[1] - w[0] <= renderer.step + 1e-6),
        "no jump"
    );
    assert_eq!(out[999], 1.0);
}

#[test]
fn zero_volume_keeps_playing() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 0.0, false);
    feed.write(&frames([0.5; 10]));
    assert!(render(&mut renderer, 10).iter().all(|&s| s == 0.0));
    assert_eq!(feed.read(), 10);
}

#[test]
fn a_flush_fades_out_drops_the_old_audio_and_fades_in() {
    let (mut feed, mut renderer) = channel(REAL, 1.0, 1.0, false);
    feed.write(&frames([1.0; 2000]));
    render(&mut renderer, 1000);
    feed.send(Control::Flush { to: feed.written() }).unwrap();
    feed.write(&frames([0.5; 2000]));
    let out = left(&render(&mut renderer, 2000));
    let fade = (1.0 / renderer.step) as usize;
    assert!(
        out.windows(2)
            .all(|w| (w[1] - w[0]).abs() <= renderer.step + 1e-6),
        "no click"
    );
    assert!(out[..fade].windows(2).all(|w| w[1] <= w[0]));
    assert_eq!(*out.last().unwrap(), 0.5);
    // The fade ends on a silent frame; everything after it is new audio, and the
    // rest of the old audio was dropped.
    let silent = out.iter().position(|&s| s == 0.0).unwrap();
    assert!(silent <= fade + 1);
    assert_eq!(feed.read(), 2000 + (out.len() - silent - 1) as u64);
}

#[test]
fn a_flush_while_paused_waits_for_play() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 1.0, true);
    feed.write(&frames([1.0; 10]));
    feed.send(Control::Flush { to: 10 }).unwrap();
    feed.write(&frames([0.5; 10]));
    assert!(render(&mut renderer, 5).iter().all(|&s| s == 0.0));
    assert_eq!(feed.read(), 10, "old audio dropped, new audio kept");
    feed.set_paused(false);
    assert_eq!(left(&render(&mut renderer, 2)), [0.5, 0.5]);
}

#[test]
fn a_cut_replaces_the_next_track_at_its_boundary() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 1.0, false);
    feed.write(&frames([0.25; 3])); // current track, frames 0..3
    feed.write(&frames([0.5; 3])); // the track that was to follow, 3..6
    feed.send(Control::Cut { from: 3, to: 6 }).unwrap();
    feed.write(&frames([0.75; 3])); // its replacement, 6..9
    assert_eq!(
        left(&render(&mut renderer, 6)),
        [0.25, 0.25, 0.25, 0.75, 0.75, 0.75]
    );
    assert_eq!(feed.read(), 9);
}

#[test]
fn cuts_at_the_same_boundary_merge() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 1.0, false);
    feed.write(&frames([0.25; 2]));
    feed.write(&frames([0.5; 2]));
    feed.send(Control::Cut { from: 2, to: 4 }).unwrap();
    feed.write(&frames([0.75; 2]));
    feed.send(Control::Cut { from: 2, to: 6 }).unwrap();
    feed.write(&frames([1.0; 2]));
    assert_eq!(left(&render(&mut renderer, 4)), [0.25, 0.25, 1.0, 1.0]);
}

#[test]
fn a_flush_overrides_a_pending_cut() {
    let (mut feed, mut renderer) = channel(INSTANT, 100.0, 1.0, false);
    feed.write(&frames([0.25; 4]));
    feed.send(Control::Cut { from: 2, to: 4 }).unwrap();
    feed.send(Control::Flush { to: 4 }).unwrap();
    feed.write(&frames([0.5; 4]));
    let out = left(&render(&mut renderer, 4));
    assert_eq!(out[1..], [0.5, 0.5, 0.5]);
}

/// Puts the ring into a state before the callback runs.
type Scenario = Box<dyn FnMut(&mut Feed)>;

#[test]
fn no_allocation_in_the_callback() {
    let (mut feed, mut renderer) = channel(REAL, 1.0, 1.0, false);
    let mut out_f32 = vec![0.0f32; 2048];
    let mut out_i16 = vec![0i16; 2048];
    let mut out_u8 = vec![0u8; 2048];
    feed.write(&vec![0.5; 48_000]);
    let mut scenarios: Vec<(&str, Scenario)> = vec![
        ("playing", Box::new(|_| {})),
        ("volume change", Box::new(|feed| feed.set_gain(0.3))),
        ("paused", Box::new(|feed| feed.set_paused(true))),
        ("resumed", Box::new(|feed| feed.set_paused(false))),
        (
            "flush",
            Box::new(|feed| {
                feed.send(Control::Flush { to: feed.written() }).unwrap();
                feed.write(&[0.25; 8192]);
            }),
        ),
        (
            "cut",
            Box::new(|feed| {
                let from = feed.read() + 600;
                feed.send(Control::Cut {
                    from,
                    to: feed.written(),
                })
                .unwrap();
                feed.write(&[0.75; 8192]);
            }),
        ),
        ("underrun", Box::new(|_| {})),
    ];
    for (name, prepare) in &mut scenarios {
        prepare(&mut feed);
        for _ in 0..30 {
            let count = allocations_in(|| {
                renderer.render(&mut out_f32);
                renderer.render(&mut out_i16);
                renderer.render(&mut out_u8);
            });
            assert_eq!(count, 0, "the callback allocated while {name}");
        }
    }
    assert_eq!(renderer.available(), 0, "the scenarios ran the ring dry");
}
