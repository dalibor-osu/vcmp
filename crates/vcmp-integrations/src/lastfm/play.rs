//! When a play counts, by Last.fm's rules: the track is longer than 30 seconds, and
//! it has played for half its length or four minutes, whichever comes first. Only
//! what was heard counts: time paused, and the part of the track a seek jumped
//! over, add nothing.

use std::time::{Duration, SystemTime};

/// Tracks this short or shorter are never scrobbled.
pub const MIN_LENGTH: Duration = Duration::from_secs(30);
/// A long track counts after this much listening, before reaching its half.
pub const MAX_WAIT: Duration = Duration::from_secs(4 * 60);
/// Positions further apart than this are a jump, not listening. The engine reports
/// the position four times a second while playing.
const MAX_STEP: Duration = Duration::from_secs(2);

/// How long a track of `duration` must play to be scrobbled; `None` when it never
/// is (too short, or of unknown length).
pub fn scrobble_point(duration: Option<Duration>) -> Option<Duration> {
    duration
        .filter(|duration| *duration > MIN_LENGTH)
        .map(|duration| (duration / 2).min(MAX_WAIT))
}

/// One play of a track, from when it starts until the next track does (a repeated
/// track is a new play). It follows the positions the engine reports.
#[derive(Clone, Debug)]
pub struct Play {
    point: Option<Duration>,
    heard: Duration,
    last: Option<Duration>,
    started: Option<SystemTime>,
    counted: bool,
}

impl Play {
    /// A play of a track `duration` long, starting at its beginning.
    pub fn new(duration: Option<Duration>) -> Self {
        Self {
            point: scrobble_point(duration),
            heard: Duration::ZERO,
            last: Some(Duration::ZERO),
            started: None,
            counted: false,
        }
    }

    /// Playback is at `position` at `now`. While `playing`, the step forward from
    /// the last position was heard; steps back or longer than two seconds are
    /// jumps. True the one time the play reaches its scrobble point.
    pub fn position(&mut self, position: Duration, playing: bool, now: SystemTime) -> bool {
        let last = self.last.replace(position);
        let step = match last {
            Some(last) if playing => position.checked_sub(last),
            _ => None,
        };
        let Some(step) = step.filter(|step| *step <= MAX_STEP && !step.is_zero()) else {
            return false;
        };
        if self.started.is_none() {
            self.started = Some(now.checked_sub(step).unwrap_or(now));
        }
        self.heard += step;
        if !self.counted && self.point.is_some_and(|point| self.heard >= point) {
            self.counted = true;
            return true;
        }
        false
    }

    /// Playback jumped (a seek): the next position starts a new stretch of
    /// listening.
    pub fn jumped(&mut self) {
        self.last = None;
    }

    /// How much of the track was heard.
    pub fn heard(&self) -> Duration {
        self.heard
    }

    /// When the first audio that was heard played.
    pub fn started(&self) -> Option<SystemTime> {
        self.started
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    const STEP: Duration = Duration::from_millis(250);

    fn secs(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    /// Plays from `from` to `to` in the engine's quarter-second steps, as heard
    /// starting at `clock`; returns how often the play reached its point, and the
    /// clock afterwards.
    fn listen(play: &mut Play, from: f64, to: f64, clock: SystemTime) -> (usize, SystemTime) {
        let (mut position, mut clock, mut reached) = (secs(from), clock, 0);
        while position < secs(to) {
            position = (position + STEP).min(secs(to));
            clock += STEP;
            if play.position(position, true, clock) {
                reached += 1;
            }
        }
        (reached, clock)
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn the_point_is_half_the_track_or_four_minutes() {
        assert_eq!(scrobble_point(Some(secs(30.0))), None);
        assert_eq!(scrobble_point(Some(secs(31.0))), Some(secs(15.5)));
        assert_eq!(scrobble_point(Some(secs(200.0))), Some(secs(100.0)));
        assert_eq!(scrobble_point(Some(secs(480.0))), Some(MAX_WAIT));
        assert_eq!(scrobble_point(Some(secs(3600.0))), Some(MAX_WAIT));
        assert_eq!(scrobble_point(None), None);
    }

    #[test]
    fn a_play_counts_once_half_of_it_was_heard() {
        let mut play = Play::new(Some(secs(200.0)));
        let (reached, _) = listen(&mut play, 0.0, 99.75, at(1000));
        assert_eq!(reached, 0);
        let (reached, _) = listen(&mut play, 99.75, 200.0, at(2000));
        assert_eq!(reached, 1, "counted once, not at every later position");
        assert_eq!(play.heard(), secs(200.0));
        assert_eq!(play.started(), Some(at(1000)));
    }

    #[test]
    fn short_tracks_never_count() {
        let mut play = Play::new(Some(secs(30.0)));
        assert_eq!(listen(&mut play, 0.0, 30.0, at(0)).0, 0);
        let mut unknown = Play::new(None);
        assert_eq!(listen(&mut unknown, 0.0, 600.0, at(0)).0, 0);
    }

    #[test]
    fn long_tracks_count_after_four_minutes() {
        let mut play = Play::new(Some(secs(3600.0)));
        assert_eq!(listen(&mut play, 0.0, 239.75, at(0)).0, 0);
        assert_eq!(listen(&mut play, 239.75, 240.0, at(0)).0, 1);
    }

    #[test]
    fn pauses_add_nothing() {
        let mut play = Play::new(Some(secs(200.0)));
        let (_, clock) = listen(&mut play, 0.0, 60.0, at(0));
        // An hour paused: the engine sends no positions, or the same one again
        // after a seek to where it stands.
        let resumed = clock + Duration::from_secs(3600);
        assert!(!play.position(secs(60.0), false, resumed));
        assert_eq!(listen(&mut play, 60.0, 99.75, resumed).0, 0);
        assert_eq!(listen(&mut play, 99.75, 100.0, resumed).0, 1);
        assert_eq!(
            play.started(),
            Some(at(0)),
            "the play started before the pause"
        );
    }

    #[test]
    fn seeking_ahead_skips_what_it_jumped_over() {
        let mut play = Play::new(Some(secs(200.0)));
        listen(&mut play, 0.0, 10.0, at(0));
        // The bar dragged to 150 s: the jump arrives as one long step.
        assert!(!play.position(secs(150.0), true, at(11)));
        assert_eq!(listen(&mut play, 150.0, 200.0, at(11)).0, 0);
        assert_eq!(play.heard(), secs(60.0));

        // A seek the service saw, even a short one, is no listening either.
        let mut play = Play::new(Some(secs(200.0)));
        listen(&mut play, 0.0, 10.0, at(0));
        play.jumped();
        assert!(!play.position(secs(11.5), true, at(11)));
        assert_eq!(play.heard(), secs(10.0));
    }

    #[test]
    fn seeking_back_counts_what_is_heard_again_but_counts_the_play_once() {
        let mut play = Play::new(Some(secs(200.0)));
        listen(&mut play, 0.0, 80.0, at(0));
        assert!(
            !play.position(secs(0.0), true, at(81)),
            "a step back is a jump"
        );
        assert_eq!(listen(&mut play, 0.0, 20.0, at(81)).0, 1);
        assert_eq!(listen(&mut play, 20.0, 200.0, at(101)).0, 0);
        assert_eq!(play.heard(), secs(280.0));
    }

    #[test]
    fn a_restored_track_counts_from_where_it_resumes() {
        let mut play = Play::new(Some(secs(200.0)));
        // Loaded paused at 150 s after a restart.
        assert!(!play.position(secs(150.0), false, at(0)));
        assert_eq!(listen(&mut play, 150.0, 200.0, at(10)).0, 0);
        assert_eq!(play.heard(), secs(50.0));
        assert_eq!(play.started(), Some(at(10)));
    }

    #[test]
    fn a_repeat_is_a_new_play() {
        let mut first = Play::new(Some(secs(60.0)));
        assert_eq!(listen(&mut first, 0.0, 60.0, at(0)).0, 1);
        let mut again = Play::new(Some(secs(60.0)));
        assert_eq!(listen(&mut again, 0.0, 60.0, at(60)).0, 1);
        assert_eq!(again.started(), Some(at(60)));
    }
}
