//! Watching source directories for changes. Copying an album produces a burst of
//! events, so the watcher waits until the directories have been quiet for a while and
//! then reports once; the caller decides when to scan.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{Result, SourceDir};

/// Stops watching when dropped.
pub struct LibraryWatcher {
    _watcher: RecommendedWatcher,
}

impl LibraryWatcher {
    /// Watches every reachable source and calls `on_change` on a background thread
    /// once no further change has arrived for `quiet_period`.
    pub fn start(
        sources: &[SourceDir],
        quiet_period: Duration,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(sender)?;
        for source in sources.iter().filter(|source| source.path.is_dir()) {
            let mode = if source.recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            watcher.watch(&source.path, mode)?;
        }
        std::thread::Builder::new()
            .name("vcmp-library-watch".into())
            .spawn(move || report_quiet_changes(&receiver, quiet_period, on_change))
            .expect("spawning the watcher thread");
        Ok(Self { _watcher: watcher })
    }
}

fn report_quiet_changes(
    receiver: &Receiver<notify::Result<Event>>,
    quiet_period: Duration,
    on_change: impl Fn(),
) {
    // The loop ends when the watcher is dropped and the channel closes.
    while let Ok(event) = receiver.recv() {
        if !is_change(&event) {
            continue;
        }
        loop {
            match receiver.recv_timeout(quiet_period) {
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        on_change();
    }
}

/// Reads are not changes; watcher errors (such as a lost event queue) might hide one.
fn is_change(event: &notify::Result<Event>) -> bool {
    match event {
        Ok(event) => !matches!(event.kind, EventKind::Access(_)),
        Err(_) => true,
    }
}
