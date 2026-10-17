use super::*;
use crate::track::TrackRef;

fn track(name: &str) -> QueueEntry {
    QueueEntry::new(TrackRef::local(format!("/music/{name}")))
}

fn album(n: usize) -> Vec<QueueEntry> {
    (0..n).map(|i| track(&format!("t{i}"))).collect()
}

fn name(entry: Option<&QueueEntry>) -> Option<String> {
    entry.map(|e| e.track.id.trim_start_matches("/music/").to_owned())
}

fn names(entries: &[QueueEntry]) -> Vec<String> {
    entries.iter().map(|e| name(Some(e)).unwrap()).collect()
}

/// Plays tracks to their end until the queue runs out or `limit` tracks played.
fn play_out(queue: &mut Queue, limit: usize) -> Vec<String> {
    let mut played: Vec<String> = name(queue.current()).into_iter().collect();
    while played.len() < limit {
        match name(queue.advance(Advance::Ended)) {
            Some(n) => played.push(n),
            None => break,
        }
    }
    played
}

fn index(name: &str) -> usize {
    name.trim_start_matches('t').parse().unwrap()
}

const SHUFFLES: [ShuffleMode; 3] = [
    ShuffleMode::Off,
    ShuffleMode::True,
    ShuffleMode::LessRepeats,
];
const REPEATS: [RepeatMode; 3] = [RepeatMode::Off, RepeatMode::All, RepeatMode::One];

#[test]
fn context_plays_in_order_and_ends() {
    let mut queue = Queue::with_seed(1);
    assert_eq!(
        name(queue.set_context(album(3), None, None)).as_deref(),
        Some("t0")
    );
    assert_eq!(play_out(&mut queue, 10), ["t0", "t1", "t2"]);
    assert!(queue.current().is_none());
    assert!(queue.advance(Advance::Ended).is_none());
}

#[test]
fn starting_mid_context_continues_from_there() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(4), Some("album:1".into()), Some(2));
    assert_eq!(queue.origin(), Some("album:1"));
    assert_eq!(play_out(&mut queue, 10), ["t2", "t3"]);
    // An out-of-range start falls back to the beginning.
    queue.set_context(album(2), None, Some(9));
    assert_eq!(play_out(&mut queue, 10), ["t0", "t1"]);
}

#[test]
fn empty_context_has_nothing_to_play() {
    for shuffle in SHUFFLES {
        for repeat in REPEATS {
            let mut queue = Queue::with_seed(1);
            queue.set_shuffle(shuffle);
            queue.set_repeat(repeat);
            assert!(queue.set_context(Vec::new(), None, None).is_none());
            assert!(queue.advance(Advance::Skipped).is_none());
            assert!(queue.peek_next().is_none());
            assert!(queue.restart().is_none());
        }
    }
}

#[test]
fn every_mode_combination_plays_what_it_promises() {
    for n in 1..=5 {
        for shuffle in SHUFFLES {
            for repeat in REPEATS {
                for seed in 0..20 {
                    let mut queue = Queue::with_seed(seed);
                    queue.set_shuffle(shuffle);
                    queue.set_repeat(repeat);
                    queue.set_context(album(n), None, None);
                    let played = play_out(&mut queue, 4 * n);
                    let indices: Vec<usize> = played.iter().map(|p| index(p)).collect();
                    let what = format!("n={n} {shuffle:?} {repeat:?} seed={seed}: {played:?}");
                    match (shuffle, repeat) {
                        (_, RepeatMode::One) => {
                            assert!(indices.iter().all(|&i| i == indices[0]), "{what}");
                            assert_eq!(played.len(), 4 * n, "{what}");
                        }
                        (ShuffleMode::True, _) => {
                            assert_eq!(played.len(), 4 * n, "true shuffle never ends: {what}");
                        }
                        (ShuffleMode::Off, RepeatMode::Off) => {
                            assert_eq!(indices, (0..n).collect::<Vec<_>>(), "{what}");
                        }
                        (ShuffleMode::Off, RepeatMode::All) => {
                            let expected: Vec<usize> = (0..4 * n).map(|i| i % n).collect();
                            assert_eq!(indices, expected, "{what}");
                        }
                        (ShuffleMode::LessRepeats, RepeatMode::Off) => {
                            let mut sorted = indices.clone();
                            sorted.sort_unstable();
                            assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "{what}");
                        }
                        (ShuffleMode::LessRepeats, RepeatMode::All) => {
                            assert_eq!(played.len(), 4 * n, "{what}");
                            for cycle in indices.chunks(n) {
                                let mut sorted = cycle.to_vec();
                                sorted.sort_unstable();
                                assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "{what}");
                            }
                            if n > 1 {
                                // Unknown artists only match the same track, so a
                                // track never plays twice in a row across cycles.
                                assert!(indices.windows(2).all(|w| w[0] != w[1]), "{what}");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn repeat_one_still_skips_and_wraps() {
    let mut queue = Queue::with_seed(1);
    queue.set_repeat(RepeatMode::One);
    queue.set_context(album(2), None, Some(1));
    assert_eq!(name(queue.advance(Advance::Ended)).as_deref(), Some("t1"));
    assert_eq!(name(queue.peek_next()).as_deref(), Some("t1"));
    assert_eq!(name(queue.advance(Advance::Skipped)).as_deref(), Some("t0"));
}

#[test]
fn true_shuffle_picks_independently() {
    let mut queue = Queue::with_seed(42);
    queue.set_shuffle(ShuffleMode::True);
    queue.set_context(album(4), None, None);
    let played = play_out(&mut queue, 40_000);
    let mut counts = [0usize; 4];
    for p in &played {
        counts[index(p)] += 1;
    }
    for count in counts {
        assert!((9_000..11_000).contains(&count), "{counts:?}");
    }
    // Independent picks repeat a track right away about a quarter of the time.
    let repeats = played.windows(2).filter(|w| w[0] == w[1]).count();
    assert!((8_000..12_000).contains(&repeats), "{repeats}");
}

#[test]
fn less_repeats_keeps_artists_apart() {
    let context: Vec<QueueEntry> = (0..12)
        .map(|i| track(&format!("t{i}")).with_artist(format!("artist {}", i % 3)))
        .collect();
    for seed in 0..50 {
        let mut queue = Queue::with_seed(seed);
        queue.set_shuffle(ShuffleMode::LessRepeats);
        queue.set_repeat(RepeatMode::All);
        queue.set_context(context.clone(), None, None);
        let played = play_out(&mut queue, 48);
        let artists: Vec<usize> = played.iter().map(|p| index(p) % 3).collect();
        assert!(
            artists.windows(2).all(|w| w[0] != w[1]),
            "seed {seed}: {played:?}"
        );
    }
}

#[test]
fn up_next_plays_first_in_order_then_the_context_resumes() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(3), None, None);
    queue.add_to_queue([track("a"), track("b")]);
    queue.play_next([track("x"), track("y")]);
    assert_eq!(
        names(&queue.view().up_next),
        ["x", "y", "a", "b"],
        "play next goes in front, keeping its own order"
    );
    assert_eq!(
        play_out(&mut queue, 20),
        ["t0", "x", "y", "a", "b", "t1", "t2"]
    );
}

#[test]
fn up_next_survives_a_new_context() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(3), None, None);
    queue.add_to_queue([track("a")]);
    queue.set_context(vec![track("n0"), track("n1")], None, Some(1));
    assert_eq!(play_out(&mut queue, 20), ["n1", "a"]);
}

#[test]
fn up_next_can_be_edited() {
    let mut queue = Queue::with_seed(1);
    queue.add_to_queue([track("a"), track("b"), track("c"), track("d")]);
    assert_eq!(name(queue.remove_up_next(1).as_ref()).as_deref(), Some("b"));
    assert!(queue.remove_up_next(10).is_none());
    queue.move_up_next(0, 2);
    queue.move_up_next(0, 10);
    assert_eq!(names(&queue.view().up_next), ["c", "d", "a"]);
    queue.clear_up_next();
    assert_eq!(queue.up_next().len(), 0);
}

#[test]
fn up_next_alone_plays_without_a_context() {
    let mut queue = Queue::with_seed(1);
    queue.add_to_queue([track("a"), track("b")]);
    assert!(queue.current().is_none());
    assert_eq!(name(queue.advance(Advance::Skipped)).as_deref(), Some("a"));
    assert_eq!(play_out(&mut queue, 10), ["a", "b"]);
}

#[test]
fn previous_walks_back_through_mixed_history_and_forward_again() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(4), None, None);
    queue.advance(Advance::Ended); // t1
    queue.add_to_queue([track("a")]);
    queue.advance(Advance::Ended); // a
    queue.advance(Advance::Ended); // t2
    assert_eq!(name(queue.previous()).as_deref(), Some("a"));
    assert_eq!(name(queue.previous()).as_deref(), Some("t1"));
    assert_eq!(name(queue.previous()).as_deref(), Some("t0"));
    assert!(
        queue.previous().is_none(),
        "no history before the first track"
    );
    assert_eq!(name(queue.current()).as_deref(), Some("t0"));
    // "a" went back to Up next, which always plays first.
    assert_eq!(play_out(&mut queue, 20), ["t0", "a", "t1", "t2", "t3"]);
}

#[test]
fn previous_then_next_returns_to_the_same_track() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(4), None, None);
    queue.advance(Advance::Ended); // t1
    queue.add_to_queue([track("a")]);
    queue.advance(Advance::Ended); // a
    queue.advance(Advance::Ended); // t2
    assert_eq!(name(queue.previous()).as_deref(), Some("a"));
    assert_eq!(name(queue.advance(Advance::Skipped)).as_deref(), Some("t2"));
    assert_eq!(name(queue.previous()).as_deref(), Some("a"));
    assert_eq!(name(queue.previous()).as_deref(), Some("t1"));
    assert_eq!(name(queue.advance(Advance::Skipped)).as_deref(), Some("a"));
}

#[test]
fn previous_after_the_end_returns_to_the_last_track() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(2), None, None);
    play_out(&mut queue, 10);
    assert_eq!(name(queue.previous()).as_deref(), Some("t1"));
    assert!(queue.advance(Advance::Ended).is_none());
}

#[test]
fn previous_returns_to_shuffled_picks() {
    for shuffle in [ShuffleMode::True, ShuffleMode::LessRepeats] {
        let mut queue = Queue::with_seed(9);
        queue.set_shuffle(shuffle);
        queue.set_repeat(RepeatMode::All);
        queue.set_context(album(6), None, None);
        let forward = play_out(&mut queue, 15);
        let mut back = Vec::new();
        while let Some(entry) = queue.previous() {
            back.push(name(Some(entry)).unwrap());
        }
        back.reverse();
        assert_eq!(back, forward[..14], "{shuffle:?}");
        // Going forward again replays the same picks.
        let again = play_out(&mut queue, 15);
        assert_eq!(again, forward, "{shuffle:?}");
    }
}

#[test]
fn peek_next_always_matches_the_next_track() {
    let mut rng = Rng::with_seed(5);
    for seed in 0..200 {
        let mut queue = Queue::with_seed(seed);
        queue.set_context(album(rng.usize(0..6)), None, None);
        for _ in 0..30 {
            match rng.usize(0..9) {
                0 => queue.set_shuffle(SHUFFLES[rng.usize(0..3)]),
                1 => queue.set_repeat(REPEATS[rng.usize(0..3)]),
                2 => queue.add_to_queue([track("q")]),
                3 => queue.play_next([track("p")]),
                4 => {
                    queue.previous();
                }
                5 => {
                    queue.view();
                }
                6 => {
                    queue.skip_to(rng.usize(0..3));
                }
                _ => {}
            }
            let peeked = queue.peek_next().cloned();
            let next = queue.advance(Advance::Ended).cloned();
            assert_eq!(peeked, next, "seed {seed}");
        }
    }
}

#[test]
fn view_matches_what_plays() {
    for shuffle in SHUFFLES {
        let mut queue = Queue::with_seed(3);
        queue.set_shuffle(shuffle);
        queue.set_context(album(8), None, None);
        queue.add_to_queue([track("a")]);
        let view = queue.view();
        let mut expected: Vec<String> = name(view.current.as_ref()).into_iter().collect();
        expected.extend(names(&view.up_next));
        expected.extend(names(&view.upcoming));
        let played = play_out(&mut queue, expected.len());
        assert_eq!(played, expected, "{shuffle:?}");
    }
}

#[test]
fn view_is_limited() {
    let mut queue = Queue::with_seed(3);
    queue.set_shuffle(ShuffleMode::True);
    queue.set_context(album(3), None, None);
    assert_eq!(queue.view().upcoming.len(), QueueView::LIMIT);
}

#[test]
fn turning_repeat_off_drops_a_cycle_shown_ahead() {
    for shuffle in [ShuffleMode::Off, ShuffleMode::LessRepeats] {
        let mut queue = Queue::with_seed(3);
        queue.set_shuffle(shuffle);
        queue.set_repeat(RepeatMode::All);
        queue.set_context(album(3), None, None);
        assert_eq!(queue.view().upcoming.len(), QueueView::LIMIT);
        queue.set_repeat(RepeatMode::Off);
        assert_eq!(queue.view().upcoming.len(), 2, "{shuffle:?}");
        assert_eq!(play_out(&mut queue, 10).len(), 3, "{shuffle:?}");
    }
}

#[test]
fn shuffle_off_continues_after_the_current_track() {
    let mut queue = Queue::with_seed(3);
    queue.set_shuffle(ShuffleMode::LessRepeats);
    queue.set_context(album(6), None, Some(3));
    queue.set_shuffle(ShuffleMode::Off);
    assert_eq!(play_out(&mut queue, 10), ["t3", "t4", "t5"]);
}

#[test]
fn shuffle_on_starts_a_cycle_without_the_current_track() {
    for seed in 0..20 {
        let mut queue = Queue::with_seed(seed);
        queue.set_context(album(6), None, Some(2));
        queue.set_shuffle(ShuffleMode::LessRepeats);
        let played = play_out(&mut queue, 10);
        let mut sorted: Vec<usize> = played.iter().map(|p| index(p)).collect();
        assert_eq!(sorted[0], 2);
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3, 4, 5], "seed {seed}: {played:?}");
    }
}

#[test]
fn skip_to_drops_the_entries_before_it() {
    let mut queue = Queue::with_seed(3);
    queue.set_context(album(5), None, None);
    queue.add_to_queue([track("a"), track("b")]);
    // Up next is a, b; upcoming is t1..t4. Index 3 is t2.
    assert_eq!(name(queue.skip_to(3)).as_deref(), Some("t2"));
    assert_eq!(
        name(queue.previous()).as_deref(),
        Some("t0"),
        "skipped entries are not history"
    );
    assert_eq!(play_out(&mut queue, 10), ["t0", "t2", "t3", "t4"]);
}

#[test]
fn restart_plays_the_context_again_after_the_end() {
    let mut queue = Queue::with_seed(3);
    queue.set_context(album(2), None, None);
    play_out(&mut queue, 10);
    assert_eq!(name(queue.restart()).as_deref(), Some("t0"));
    assert_eq!(play_out(&mut queue, 10), ["t0", "t1"]);
    // While something is current, restart changes nothing.
    queue.set_context(album(2), None, Some(1));
    assert_eq!(name(queue.restart()).as_deref(), Some("t1"));
}

#[test]
fn snapshots_round_trip() {
    let mut rng = Rng::with_seed(8);
    for seed in 0..100 {
        let mut queue = Queue::with_seed(seed);
        queue.set_shuffle(SHUFFLES[rng.usize(0..3)]);
        queue.set_repeat(REPEATS[rng.usize(0..3)]);
        queue.set_context(album(rng.usize(1..8)), Some("playlist:3".into()), None);
        queue.add_to_queue([track("a"), track("b")]);
        for _ in 0..rng.usize(0..6) {
            queue.advance(Advance::Skipped);
        }
        let snapshot = queue.snapshot();
        let mut restored = Queue::from_snapshot(snapshot.clone()).unwrap();
        assert_eq!(restored.snapshot(), snapshot);
        assert_eq!(restored.current(), queue.current());
        // What was already decided plays the same after a restore.
        let decided = 1 + snapshot.up_next.len() + snapshot.order.len() - snapshot.cursor;
        let before = play_out(&mut queue, decided);
        let after = play_out(&mut restored, decided);
        assert_eq!(after, before, "seed {seed}");
        assert_eq!(restored.previous(), queue.previous(), "seed {seed}");
    }
}

#[test]
fn invalid_snapshots_are_rejected() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(3), None, None);
    let good = queue.snapshot();
    let broken = [
        QueueSnapshot {
            order: vec![0, 5],
            ..good.clone()
        },
        QueueSnapshot {
            cursor: 9,
            ..good.clone()
        },
        QueueSnapshot {
            current: Some(QueueSlot::Context(2)),
            ..good.clone()
        },
        QueueSnapshot {
            history: vec![QueueSlot::Context(7)],
            ..good.clone()
        },
    ];
    for snapshot in broken {
        assert_eq!(Queue::from_snapshot(snapshot).unwrap_err(), InvalidSnapshot);
    }
    assert!(Queue::from_snapshot(good).is_ok());
    assert!(Queue::from_snapshot(QueueSnapshot::default()).is_ok());
}

#[test]
fn endless_play_keeps_memory_bounded() {
    for shuffle in [
        ShuffleMode::True,
        ShuffleMode::LessRepeats,
        ShuffleMode::Off,
    ] {
        let mut queue = Queue::with_seed(2);
        queue.set_shuffle(shuffle);
        queue.set_repeat(RepeatMode::All);
        queue.set_context(album(10), None, None);
        for _ in 0..50_000 {
            queue.advance(Advance::Ended);
        }
        assert!(
            queue.order.len() < COMPACT_AFTER + 2 * HISTORY_LIMIT + 20,
            "{shuffle:?}"
        );
        assert!(queue.history.len() <= HISTORY_LIMIT);
        let current = queue.current().cloned();
        queue.previous();
        assert_eq!(
            queue.advance(Advance::Skipped).cloned(),
            current,
            "{shuffle:?}"
        );
    }
}

#[test]
fn drop_next_skips_what_would_play_next() {
    let mut queue = Queue::with_seed(1);
    queue.set_context(album(4), None, None);
    queue.add_to_queue([track("a")]);
    assert!(queue.drop_next());
    assert_eq!(name(queue.peek_next()).as_deref(), Some("t1"));
    assert!(queue.drop_next());
    assert_eq!(
        name(queue.current()).as_deref(),
        Some("t0"),
        "the current track stays"
    );
    assert_eq!(play_out(&mut queue, 10), ["t0", "t2", "t3"]);
    assert!(!queue.drop_next(), "nothing left");
    queue.set_context(album(2), None, None);
    queue.set_repeat(RepeatMode::One);
    assert!(!queue.drop_next(), "repeat one has no other next track");
}
