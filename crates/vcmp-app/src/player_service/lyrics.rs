//! The playing track's lyrics in `PlayerState`, for the now playing view. Local
//! lyrics (the `.lrc` file and the tags) are read off the UI thread when a track
//! starts. When none of them is synced and "Find lyrics online" is on, the library's
//! cache of LRCLIB answers is consulted and LRCLIB asked when the cache has no
//! answer to the track's current tags, as the cover lookup does for Discord. The line
//! playing follows the engine's positions, and between them the clock: each line
//! change is timed for the moment its line starts.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use slint::{ComponentHandle, ModelRc, SharedString, TimerMode, VecModel};
use vcmp_integrations::lrclib::{self, Answer, LyricsFinder};
use vcmp_library::{
    LocalLyrics, Lyrics, LyricsKey, LyricsQuery, LyricsText, OnlineLyrics, Track, TrackId,
    choose_lyrics, lyrics_lookup_due, read_local_lyrics,
};

use super::{PlayerService, RETRY_FAILED, SERVICE};
use crate::PlayerState;

/// Setting key: ask LRCLIB for lyrics ("true" or "false"; absent: false).
pub const SETTING_LYRICS_ONLINE: &str = "integrations.lyrics_online";
/// Setting key: the now playing view shows lyrics instead of the cover when the
/// track has some ("true" or "false"; absent: false).
pub const SETTING_LYRICS_SHOWN: &str = "ui.lyrics_shown";

/// Where a track's local lyrics are read: a thread per track, or a recorder in tests.
/// The lyrics come back through `PlayerService::local_lyrics_read`.
pub trait LyricsReader {
    fn read(&self, path: PathBuf);
}

/// Reads on a thread of its own and hands the lyrics to the UI thread.
pub struct ThreadReader;

impl LyricsReader for ThreadReader {
    fn read(&self, path: PathBuf) {
        let spawned = thread::Builder::new()
            .name("vcmp-lyrics-read".into())
            .spawn(move || {
                let remote = path
                    .to_str()
                    .and_then(vcmp_library::parse_remote_key)
                    .is_some();
                let local = if remote {
                    LocalLyrics::default()
                } else {
                    read_local_lyrics(&path)
                };
                PlayerService::post(move |service| service.local_lyrics_read(&path, local));
            });
        if let Err(error) = spawned {
            log::warn!("could not start reading lyrics: {error}");
        }
    }
}

/// Where lyrics lookups go: LRCLIB, or a recorder in tests. The answer comes back
/// through `PlayerService::lyrics_found`.
pub trait LyricsSearch {
    fn find(&self, track: TrackId, query: lrclib::LyricsQuery);
}

impl LyricsSearch for LyricsFinder<TrackId> {
    fn find(&self, track: TrackId, query: lrclib::LyricsQuery) {
        LyricsFinder::find(self, track, query);
    }
}

/// The track whose lyrics are shown or on their way.
struct Current {
    path: PathBuf,
    /// The library's track, which LRCLIB can be asked about, and the question.
    track: Option<(TrackId, LyricsQuery)>,
    /// `None` while they are read.
    local: Option<LocalLyrics>,
    /// LRCLIB was asked and has not answered.
    looking_up: bool,
}

pub(super) struct LyricsState {
    reader: RefCell<Box<dyn LyricsReader>>,
    search: RefCell<Option<Box<dyn LyricsSearch>>>,
    current: RefCell<Option<Current>>,
    shown: RefCell<Option<Lyrics>>,
    /// The start of each synced line shown.
    starts: RefCell<Vec<Duration>>,
    line: Cell<i32>,
    /// Tracks whose lookup failed, and when they may be asked again.
    failed: RefCell<HashMap<TrackId, Instant>>,
    /// The last position known, when it was, and whether the clock runs from it.
    position: Cell<Duration>,
    position_at: Cell<Instant>,
    playing: Cell<bool>,
    /// Fires when the next line starts.
    timer: slint::Timer,
}

impl Default for LyricsState {
    fn default() -> Self {
        Self {
            reader: RefCell::new(Box::new(ThreadReader)),
            search: RefCell::default(),
            current: RefCell::default(),
            shown: RefCell::default(),
            starts: RefCell::default(),
            line: Cell::new(-1),
            failed: RefCell::default(),
            position: Cell::default(),
            position_at: Cell::new(Instant::now()),
            playing: Cell::new(false),
            timer: slint::Timer::default(),
        }
    }
}

/// The question LRCLIB is asked about `track`; none without an artist.
fn query_for(track: &Track, duration: Duration) -> Option<LyricsQuery> {
    Some(LyricsQuery {
        title: track.title.clone(),
        artist: track.artist.clone()?,
        album: track.album.clone(),
        duration_secs: duration.as_secs_f64().round() as u32,
    })
}

impl PlayerService {
    /// Reads local lyrics with `reader` from now on instead of on a thread (tests).
    pub fn set_lyrics_reader(&self, reader: Box<dyn LyricsReader>) {
        *self.lyrics.reader.borrow_mut() = reader;
    }

    /// Asks `search` for lyrics online from now on, while the setting allows it.
    pub fn set_lyrics_search(&self, search: Box<dyn LyricsSearch>) {
        *self.lyrics.search.borrow_mut() = Some(search);
        self.resolve_lyrics();
    }

    pub(super) fn connect_lyrics(&self, state: &PlayerState) {
        state.set_lyrics_online(self.setting(SETTING_LYRICS_ONLINE).as_deref() == Some("true"));
        state.set_lyrics_shown(self.setting(SETTING_LYRICS_SHOWN).as_deref() == Some("true"));
        state.on_set_lyrics_online(|enabled| {
            if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                service.set_lyrics_online(enabled);
            }
        });
        state.on_set_lyrics_shown(|shown| {
            if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                service.set_lyrics_shown(shown);
            }
        });
        state.on_seek_to_lyric(|index| {
            if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                service.seek_to_lyric(index);
            }
        });
    }

    /// A track started (or none plays): forgets the lyrics shown and reads its own.
    pub(super) fn lyrics_track_started(
        &self,
        path: Option<&Path>,
        track: Option<&Track>,
        duration: Duration,
    ) {
        *self.lyrics.current.borrow_mut() = path.map(|path| Current {
            path: path.to_path_buf(),
            track: track.and_then(|track| Some((track.id, query_for(track, duration)?))),
            local: None,
            looking_up: false,
        });
        self.show_lyrics(None);
        self.lyrics_position(Duration::ZERO);
        if let Some(path) = path {
            self.lyrics.reader.borrow().read(path.to_path_buf());
        }
    }

    /// The playing track's tags changed: reads its lyrics again and asks LRCLIB the
    /// new question if it needs to, keeping what is shown meanwhile.
    pub(super) fn lyrics_tags_changed(&self, track: &Track) {
        let path = {
            let mut current = self.lyrics.current.borrow_mut();
            let Some(current) = current.as_mut() else {
                return;
            };
            current.track = query_for(track, track.duration).map(|query| (track.id, query));
            current.path.clone()
        };
        self.lyrics.reader.borrow().read(path);
    }

    /// Takes the local lyrics of the track at `path`, read off the UI thread.
    pub fn local_lyrics_read(&self, path: &Path, local: LocalLyrics) {
        {
            let mut current = self.lyrics.current.borrow_mut();
            match current.as_mut() {
                Some(current) if current.path == path => current.local = Some(local),
                _ => return,
            }
        }
        self.resolve_lyrics();
    }

    /// Takes LRCLIB's answer for `track` to `query`: stores it, and shows lyrics it
    /// found if the track still plays.
    pub fn lyrics_found(&self, track: TrackId, query: lrclib::LyricsQuery, answer: Answer) {
        let asked = LyricsQuery {
            title: query.title,
            artist: query.artist,
            album: query.album,
            duration_secs: query.duration.as_secs_f64().round() as u32,
        };
        let found = match answer {
            Answer::Found { synced, plain } => Some(OnlineLyrics { synced, plain }),
            Answer::NotFound => None,
            Answer::Failed { error, retry_after } => {
                log::info!("the lyrics lookup failed: {error}");
                let wait = retry_after.map_or(RETRY_FAILED, |after| after.max(RETRY_FAILED));
                self.lyrics
                    .failed
                    .borrow_mut()
                    .insert(track, Instant::now() + wait);
                self.lyrics_answered(track);
                return;
            }
        };
        if let Err(error) = self.library.set_lyrics_lookup(
            &LyricsKey::local(track),
            &asked,
            found.as_ref(),
            SystemTime::now(),
        ) {
            log::warn!("could not store the lyrics lookup: {error}");
        }
        self.lyrics_answered(track);
    }

    fn lyrics_answered(&self, track: TrackId) {
        let playing = {
            let mut current = self.lyrics.current.borrow_mut();
            match current.as_mut() {
                Some(current) if current.track.as_ref().is_some_and(|(id, _)| *id == track) => {
                    current.looking_up = false;
                    true
                }
                _ => false,
            }
        };
        if playing {
            self.resolve_lyrics();
        }
    }

    fn lyrics_online(&self) -> bool {
        self.window
            .upgrade()
            .is_some_and(|window| window.global::<PlayerState>().get_lyrics_online())
    }

    /// Shows the best lyrics known for the playing track, and asks LRCLIB when
    /// nothing synced is local and the cache has no answer to the track's tags.
    fn resolve_lyrics(&self) {
        let (local, track, looking_up) = {
            let current = self.lyrics.current.borrow();
            let Some(current) = current.as_ref() else {
                return;
            };
            let Some(local) = current.local.clone() else {
                return;
            };
            (local, current.track.clone(), current.looking_up)
        };
        let mut online = None;
        if !local.has_synced()
            && self.lyrics_online()
            && let Some((id, query)) = &track
        {
            let stored = self
                .library
                .lyrics_lookup(&LyricsKey::local(*id))
                .unwrap_or_else(|error| {
                    log::warn!("could not read the lyrics lookup: {error}");
                    None
                });
            if let Some(lookup) = stored.as_ref().filter(|lookup| lookup.answers(query)) {
                online = lookup.found.clone();
            }
            let failed_lately = self
                .lyrics
                .failed
                .borrow()
                .get(id)
                .is_some_and(|until| Instant::now() < *until);
            if !looking_up
                && !failed_lately
                && lyrics_lookup_due(stored.as_ref(), query)
                && let Some(search) = self.lyrics.search.borrow().as_ref()
            {
                if let Some(current) = self.lyrics.current.borrow_mut().as_mut() {
                    current.looking_up = true;
                }
                search.find(
                    *id,
                    lrclib::LyricsQuery {
                        title: query.title.clone(),
                        artist: query.artist.clone(),
                        album: query.album.clone(),
                        duration: Duration::from_secs(u64::from(query.duration_secs)),
                    },
                );
            }
        }
        self.show_lyrics(choose_lyrics(&local, online.as_ref()));
    }

    fn show_lyrics(&self, lyrics: Option<Lyrics>) {
        if *self.lyrics.shown.borrow() == lyrics {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        let (lines, starts, text) = match lyrics.as_ref().map(|lyrics| &lyrics.text) {
            Some(LyricsText::Synced(lines)) => (
                lines
                    .iter()
                    .map(|line| SharedString::from(line.text.as_str()))
                    .collect(),
                lines.iter().map(|line| line.start).collect(),
                SharedString::default(),
            ),
            Some(LyricsText::Plain(text)) => {
                (Vec::new(), Vec::new(), SharedString::from(text.as_str()))
            }
            None => (Vec::new(), Vec::new(), SharedString::default()),
        };
        state.set_lyrics_synced(!starts.is_empty());
        state.set_lyrics_lines(ModelRc::new(VecModel::from(lines)));
        state.set_lyrics_text(text);
        state.set_has_lyrics(lyrics.is_some());
        *self.lyrics.starts.borrow_mut() = starts;
        *self.lyrics.shown.borrow_mut() = lyrics;
        self.lyrics.line.set(-1);
        state.set_lyrics_current(-1);
        self.follow_lyrics();
    }

    /// The engine reported `position`.
    pub(super) fn lyrics_position(&self, position: Duration) {
        self.lyrics.position.set(position);
        self.lyrics.position_at.set(Instant::now());
        self.follow_lyrics();
    }

    /// Playback started or stopped running.
    pub(super) fn lyrics_playing(&self, playing: bool) {
        let now = self.lyrics_now();
        self.lyrics.position.set(now);
        self.lyrics.position_at.set(Instant::now());
        self.lyrics.playing.set(playing);
        self.follow_lyrics();
    }

    /// Where playback is now: the last position plus the time since, while playing.
    fn lyrics_now(&self) -> Duration {
        let position = self.lyrics.position.get();
        if self.lyrics.playing.get() {
            position + self.lyrics.position_at.get().elapsed()
        } else {
            position
        }
    }

    /// Marks the line playing now, and times the change to the next one.
    fn follow_lyrics(&self) {
        let now = self.lyrics_now();
        let starts = self.lyrics.starts.borrow();
        let line = starts.partition_point(|start| *start <= now) as i32 - 1;
        if line != self.lyrics.line.get() {
            self.lyrics.line.set(line);
            if let Some(window) = self.window.upgrade() {
                window.global::<PlayerState>().set_lyrics_current(line);
            }
        }
        let next = starts.get((line + 1) as usize).copied();
        drop(starts);
        match next {
            Some(next) if self.lyrics.playing.get() => {
                self.lyrics.timer.start(
                    TimerMode::SingleShot,
                    next.saturating_sub(now),
                    move || {
                        if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                            // The timer was set for this moment.
                            service.lyrics.position.set(next);
                            service.lyrics.position_at.set(Instant::now());
                            service.follow_lyrics();
                        }
                    },
                );
            }
            _ => self.lyrics.timer.stop(),
        }
    }

    fn seek_to_lyric(&self, index: i32) {
        let start = usize::try_from(index)
            .ok()
            .and_then(|index| self.lyrics.starts.borrow().get(index).copied());
        if let Some(start) = start {
            self.seek(start);
            self.lyrics_position(start);
        }
    }

    fn set_lyrics_online(&self, enabled: bool) {
        if let Some(window) = self.window.upgrade() {
            window.global::<PlayerState>().set_lyrics_online(enabled);
        }
        self.save(
            SETTING_LYRICS_ONLINE,
            if enabled { "true" } else { "false" },
        );
        self.resolve_lyrics();
    }

    fn set_lyrics_shown(&self, shown: bool) {
        if let Some(window) = self.window.upgrade() {
            window.global::<PlayerState>().set_lyrics_shown(shown);
        }
        self.save(SETTING_LYRICS_SHOWN, if shown { "true" } else { "false" });
    }
}
