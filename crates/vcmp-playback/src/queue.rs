//! The play queue: a context (the album, playlist or list the user started) plus a
//! manual Up next list that always plays first, in order.
//!
//! The context keeps its original order in `context`; `order` lists context indices
//! in play order, which is where shuffle lives. `order[..cursor]` has been played or
//! skipped, `order[cursor..]` is the future. Positions in `order` stay valid while
//! the future is regenerated (shuffle changes) or extended (repeat, true shuffle),
//! so history entries can point into it.

use std::collections::VecDeque;
use std::fmt;

use fastrand::Rng;

use crate::shuffle::{Groups, spread};
use crate::track::QueueEntry;

/// How the context is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShuffleMode {
    /// The context's own order.
    #[default]
    Off,
    /// Every next track is an independent random pick from the whole context, so a
    /// track can come back soon (even right away), and playback never runs out.
    True,
    /// A permutation per cycle (each track once), keeping the same artist and the
    /// same album out of adjacent slots where possible.
    LessRepeats,
}

/// What happens at the end of a track or of the context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RepeatMode {
    /// Stop after the last track of the context.
    #[default]
    Off,
    /// Start the context again (a new cycle when shuffling).
    All,
    /// Play the current track again when it ends. Skipping still moves on, and the
    /// context wraps around as with `All`.
    One,
}

/// Why the queue moves to the next track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// The current track played to its end; [`RepeatMode::One`] keeps it.
    Ended,
    /// The user skipped it.
    Skipped,
}

/// Where a track in the queue came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueSlot {
    /// A context track, by its position in [`QueueSnapshot::order`].
    Context(usize),
    /// A track that was taken from Up next.
    UpNext(QueueEntry),
}

/// Everything needed to rebuild a [`Queue`], for keeping it across restarts.
///
/// Plain data: persist it in whatever form suits the store and pass it back to
/// [`Queue::from_snapshot`] (or the player's restore command).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub context: Vec<QueueEntry>,
    pub origin: Option<String>,
    pub order: Vec<usize>,
    pub cursor: usize,
    pub current: Option<QueueSlot>,
    pub history: Vec<QueueSlot>,
    pub up_next: Vec<QueueEntry>,
    pub shuffle: ShuffleMode,
    pub repeat: RepeatMode,
}

/// A [`QueueSnapshot`] whose indices do not fit its context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidSnapshot;

impl fmt::Display for InvalidSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the saved queue does not match its context")
    }
}

impl std::error::Error for InvalidSnapshot {}

/// What the UI shows in the queue panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueView {
    pub current: Option<QueueEntry>,
    pub up_next: Vec<QueueEntry>,
    /// Context tracks after the current one, in play order, at most
    /// [`QueueView::LIMIT`] of them.
    pub upcoming: Vec<QueueEntry>,
    /// The app's own key for the context ("album:42", ...), as given when it was set.
    pub origin: Option<String>,
    pub shuffle: ShuffleMode,
    pub repeat: RepeatMode,
}

impl QueueView {
    pub const LIMIT: usize = 100;
}

/// History entries kept for "previous".
const HISTORY_LIMIT: usize = 100;
/// Played positions in `order` kept before they are dropped.
const COMPACT_AFTER: usize = 4096;

#[derive(Debug)]
pub struct Queue {
    context: Vec<QueueEntry>,
    origin: Option<String>,
    groups: Groups,
    order: Vec<usize>,
    cursor: usize,
    /// Where `order` was extended ahead of time for another repeat cycle. Turning
    /// repeat off drops everything from here on.
    repeat_from: Option<usize>,
    up_next: VecDeque<QueueEntry>,
    current: Option<QueueSlot>,
    history: VecDeque<QueueSlot>,
    shuffle: ShuffleMode,
    repeat: RepeatMode,
    rng: Rng,
}

impl Default for Queue {
    fn default() -> Self {
        Queue::with_rng(Rng::new())
    }
}

impl Queue {
    pub fn new() -> Self {
        Queue::default()
    }

    /// A queue whose shuffles are reproducible.
    pub fn with_seed(seed: u64) -> Self {
        Queue::with_rng(Rng::with_seed(seed))
    }

    fn with_rng(rng: Rng) -> Self {
        Queue {
            context: Vec::new(),
            origin: None,
            groups: Groups::default(),
            order: Vec::new(),
            cursor: 0,
            repeat_from: None,
            up_next: VecDeque::new(),
            current: None,
            history: VecDeque::new(),
            shuffle: ShuffleMode::Off,
            repeat: RepeatMode::Off,
            rng,
        }
    }

    pub fn current(&self) -> Option<&QueueEntry> {
        self.current.as_ref().map(|slot| self.entry(slot))
    }

    pub fn context(&self) -> &[QueueEntry] {
        &self.context
    }

    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub fn up_next(&self) -> impl ExactSizeIterator<Item = &QueueEntry> {
        self.up_next.iter()
    }

    pub fn shuffle(&self) -> ShuffleMode {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    /// Whether [`previous`](Self::previous) has a track to go back to.
    pub fn has_history(&self) -> bool {
        !self.history.is_empty()
    }

    /// Replaces the context and makes `start` (an index into `entries`) current, or
    /// the first track in play order when `start` is `None` or out of range.
    ///
    /// Up next is kept and plays after the new current track. History is cleared.
    pub fn set_context(
        &mut self,
        entries: Vec<QueueEntry>,
        origin: Option<String>,
        start: Option<usize>,
    ) -> Option<&QueueEntry> {
        self.groups = Groups::new(&entries);
        self.context = entries;
        self.origin = origin;
        self.order.clear();
        self.cursor = 0;
        self.repeat_from = None;
        self.history.clear();
        self.current = None;
        let n = self.context.len();
        if n == 0 {
            return None;
        }
        let start = start.filter(|&s| s < n);
        match self.shuffle {
            ShuffleMode::Off => {
                self.order.extend(0..n);
                self.cursor = start.unwrap_or(0);
            }
            ShuffleMode::True => {
                let first = start.unwrap_or_else(|| self.rng.usize(..n));
                self.order.push(first);
            }
            ShuffleMode::LessRepeats => match start {
                Some(first) => {
                    let rest: Vec<usize> = (0..n).filter(|&i| i != first).collect();
                    self.order.push(first);
                    let cycle = spread(&rest, &self.groups, Some(first), &mut self.rng);
                    self.order.extend(cycle);
                }
                None => {
                    let all: Vec<usize> = (0..n).collect();
                    self.order = spread(&all, &self.groups, None, &mut self.rng);
                }
            },
        }
        self.current = Some(QueueSlot::Context(self.cursor));
        self.cursor += 1;
        self.current()
    }

    /// Puts tracks at the front of Up next, keeping their order.
    pub fn play_next(&mut self, entries: impl IntoIterator<Item = QueueEntry>) {
        let entries: Vec<QueueEntry> = entries.into_iter().collect();
        for entry in entries.into_iter().rev() {
            self.up_next.push_front(entry);
        }
    }

    /// Appends tracks to Up next.
    pub fn add_to_queue(&mut self, entries: impl IntoIterator<Item = QueueEntry>) {
        self.up_next.extend(entries);
    }

    pub fn remove_up_next(&mut self, index: usize) -> Option<QueueEntry> {
        self.up_next.remove(index)
    }

    /// Moves an Up next entry; out-of-range indices are ignored.
    pub fn move_up_next(&mut self, from: usize, to: usize) {
        if from < self.up_next.len()
            && to < self.up_next.len()
            && let Some(entry) = self.up_next.remove(from)
        {
            self.up_next.insert(to, entry);
        }
    }

    pub fn clear_up_next(&mut self) {
        self.up_next.clear();
    }

    /// Moves to the next track: Up next first, then the context. Returns the new
    /// current track, or `None` when the queue has run out.
    pub fn advance(&mut self, why: Advance) -> Option<&QueueEntry> {
        if why == Advance::Ended && self.repeat == RepeatMode::One && self.current.is_some() {
            return self.current();
        }
        if let Some(slot) = self.current.take() {
            self.history.push_back(slot);
            if self.history.len() > HISTORY_LIMIT {
                self.history.pop_front();
            }
        }
        if let Some(entry) = self.up_next.pop_front() {
            self.current = Some(QueueSlot::UpNext(entry));
        } else if self.context_has(self.cursor) {
            self.current = Some(QueueSlot::Context(self.cursor));
            self.cursor += 1;
            if self.repeat_from.is_some_and(|from| self.cursor > from) {
                self.repeat_from = None;
            }
        }
        self.compact();
        self.current()
    }

    /// The track that [`advance`](Self::advance) with [`Advance::Ended`] would make
    /// current. A true-shuffle pick drawn here stays fixed.
    pub fn peek_next(&mut self) -> Option<&QueueEntry> {
        if self.repeat == RepeatMode::One && self.current.is_some() {
            return self.current();
        }
        if !self.up_next.is_empty() {
            return self.up_next.front();
        }
        if self.context_has(self.cursor) {
            return Some(&self.context[self.order[self.cursor]]);
        }
        None
    }

    /// Removes the track [`peek_next`](Self::peek_next) returns, without touching
    /// the current one: for a next track that turned out unplayable. False when
    /// there is nothing to remove, or when the next track is the current one again
    /// ([`RepeatMode::One`]).
    pub fn drop_next(&mut self) -> bool {
        if self.repeat == RepeatMode::One && self.current.is_some() {
            return false;
        }
        if self.up_next.pop_front().is_some() {
            return true;
        }
        if !self.context_has(self.cursor) {
            return false;
        }
        self.cursor += 1;
        if self.repeat_from.is_some_and(|from| self.cursor > from) {
            self.repeat_from = None;
        }
        true
    }

    /// Goes back to the previously played track. The track left behind returns to
    /// where it came from: the front of Up next, or the front of the context's
    /// upcoming tracks. Up next still plays first, so going back over a mix of both
    /// and forward again plays the Up next tracks earlier. `None` when there is no
    /// history; the caller then restarts the current track instead.
    pub fn previous(&mut self) -> Option<&QueueEntry> {
        let past = self.history.pop_back()?;
        match self.current.take() {
            Some(QueueSlot::UpNext(entry)) => self.up_next.push_front(entry),
            Some(QueueSlot::Context(position)) => self.cursor = position,
            None => {}
        }
        self.current = Some(past);
        self.current()
    }

    /// After the queue ran out: starts the context over from its beginning (a new
    /// cycle when shuffling).
    pub fn restart(&mut self) -> Option<&QueueEntry> {
        if self.current.is_some() || self.context.is_empty() {
            return self.current();
        }
        self.order.truncate(self.cursor);
        self.repeat_from = None;
        self.extend_cycle();
        self.advance(Advance::Skipped)
    }

    /// Jumps to an entry of the upcoming list, counted over Up next followed by
    /// [`QueueView::upcoming`]. The entries before it are dropped without becoming
    /// history.
    pub fn skip_to(&mut self, index: usize) -> Option<&QueueEntry> {
        for _ in 0..index {
            if self.up_next.pop_front().is_some() {
                continue;
            }
            if !self.context_has(self.cursor) {
                break;
            }
            self.cursor += 1;
        }
        self.advance(Advance::Skipped)
    }

    pub fn set_shuffle(&mut self, mode: ShuffleMode) {
        if mode == self.shuffle {
            return;
        }
        self.shuffle = mode;
        self.order.truncate(self.cursor);
        self.repeat_from = None;
        let n = self.context.len();
        let current = self.current_context_index();
        match mode {
            // Continue in context order after the current (or last played) track.
            ShuffleMode::Off => {
                let last = current.or_else(|| self.cursor.checked_sub(1).map(|p| self.order[p]));
                self.order.extend(last.map_or(0, |i| i + 1)..n);
            }
            // Drawn lazily, one pick at a time.
            ShuffleMode::True => {}
            // A new cycle of everything except the current track.
            ShuffleMode::LessRepeats => {
                let rest: Vec<usize> = (0..n).filter(|&i| Some(i) != current).collect();
                let cycle = spread(&rest, &self.groups, current, &mut self.rng);
                self.order.extend(cycle);
            }
        }
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
        if mode == RepeatMode::Off
            && let Some(from) = self.repeat_from.take()
        {
            self.order.truncate(from);
        }
    }

    /// The current track, Up next and the next context tracks.
    pub fn view(&mut self) -> QueueView {
        let mut upcoming = Vec::new();
        let mut position = self.cursor;
        while upcoming.len() < QueueView::LIMIT && self.context_has(position) {
            upcoming.push(self.context[self.order[position]].clone());
            position += 1;
        }
        QueueView {
            current: self.current().cloned(),
            up_next: self.up_next.iter().cloned().collect(),
            upcoming,
            origin: self.origin.clone(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        }
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        let order_len = self.repeat_from.unwrap_or(self.order.len());
        QueueSnapshot {
            context: self.context.clone(),
            origin: self.origin.clone(),
            order: self.order[..order_len].to_vec(),
            cursor: self.cursor,
            current: self.current.clone(),
            history: self.history.iter().cloned().collect(),
            up_next: self.up_next.iter().cloned().collect(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        }
    }

    pub fn from_snapshot(snapshot: QueueSnapshot) -> Result<Queue, InvalidSnapshot> {
        let QueueSnapshot {
            context,
            origin,
            order,
            cursor,
            current,
            history,
            up_next,
            shuffle,
            repeat,
        } = snapshot;
        let valid_slot = |slot: &QueueSlot, below: usize| match slot {
            QueueSlot::Context(position) => *position < below,
            QueueSlot::UpNext(_) => true,
        };
        let valid = order.iter().all(|&i| i < context.len())
            && cursor <= order.len()
            && current.as_ref().is_none_or(|slot| valid_slot(slot, cursor))
            && history.iter().all(|slot| valid_slot(slot, order.len()));
        if !valid {
            return Err(InvalidSnapshot);
        }
        let mut queue = Queue::new();
        queue.groups = Groups::new(&context);
        queue.context = context;
        queue.origin = origin;
        queue.order = order;
        queue.cursor = cursor;
        queue.current = current;
        queue.history = history.into();
        queue.up_next = up_next.into();
        queue.shuffle = shuffle;
        queue.repeat = repeat;
        Ok(queue)
    }

    fn entry<'a>(&'a self, slot: &'a QueueSlot) -> &'a QueueEntry {
        match slot {
            QueueSlot::Context(position) => &self.context[self.order[*position]],
            QueueSlot::UpNext(entry) => entry,
        }
    }

    fn current_context_index(&self) -> Option<usize> {
        match self.current {
            Some(QueueSlot::Context(position)) => Some(self.order[position]),
            _ => None,
        }
    }

    /// Whether `order[position]` exists, extending the order if the modes allow it.
    fn context_has(&mut self, position: usize) -> bool {
        while position >= self.order.len() {
            if !self.extend() {
                return false;
            }
        }
        true
    }

    /// Appends to the context's future; `false` when the context has ended.
    fn extend(&mut self) -> bool {
        let n = self.context.len();
        if n == 0 {
            return false;
        }
        if self.shuffle == ShuffleMode::True {
            let pick = self.rng.usize(..n);
            self.order.push(pick);
            return true;
        }
        if self.repeat == RepeatMode::Off {
            return false;
        }
        if self.repeat_from.is_none() {
            self.repeat_from = Some(self.order.len());
        }
        self.extend_cycle();
        true
    }

    fn extend_cycle(&mut self) {
        let n = self.context.len();
        match self.shuffle {
            ShuffleMode::Off => self.order.extend(0..n),
            ShuffleMode::True => self.order.push(self.rng.usize(..n)),
            ShuffleMode::LessRepeats => {
                let all: Vec<usize> = (0..n).collect();
                let previous = self.order.last().copied();
                let cycle = spread(&all, &self.groups, previous, &mut self.rng);
                self.order.extend(cycle);
            }
        }
    }

    /// Drops played positions nothing refers to any more, so endless shuffle and
    /// repeat do not grow `order` forever.
    fn compact(&mut self) {
        let referenced = self
            .history
            .iter()
            .chain(&self.current)
            .filter_map(|slot| match slot {
                QueueSlot::Context(position) => Some(*position),
                QueueSlot::UpNext(_) => None,
            })
            .min()
            .unwrap_or(self.cursor)
            .min(self.cursor);
        if referenced < COMPACT_AFTER {
            return;
        }
        self.order.drain(..referenced);
        self.cursor -= referenced;
        if let Some(from) = &mut self.repeat_from {
            *from -= referenced;
        }
        for slot in self.history.iter_mut().chain(&mut self.current) {
            if let QueueSlot::Context(position) = slot {
                *position -= referenced;
            }
        }
    }
}

#[cfg(test)]
mod tests;
