//! Watching source directories for changes. Copying an album produces a burst of
//! events, so the watcher waits until the directories have been quiet for a while and
//! then reports once; the caller decides when to scan.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{Result, SourceDir};

/// Stops watching when dropped.
pub struct LibraryWatcher {
    _watcher: RecommendedWatcher,
}

impl LibraryWatcher {
    /// Watches every reachable source and calls `on_change` on a background thread
    /// once no further change has arrived for `quiet_period`, or at the latest
    /// [`LONGEST_WAIT_IN_QUIET_PERIODS`] quiet periods after the first change, so a
    /// sync tool busy in a folder can't put the scan off for as long as it runs.
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

pub const LONGEST_WAIT_IN_QUIET_PERIODS: u32 = 30;

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
        let deadline = Instant::now() + quiet_period * LONGEST_WAIT_IN_QUIET_PERIODS;
        loop {
            let wait = quiet_period.min(deadline.saturating_duration_since(Instant::now()));
            if wait.is_zero() {
                break;
            }
            match receiver.recv_timeout(wait) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_that_never_stop_are_still_reported() {
        let (sender, receiver) = mpsc::channel();
        let quiet = Duration::from_millis(20);
        let busy = std::thread::spawn(move || {
            for _ in 0..600 {
                let event = Event::new(EventKind::Create(notify::event::CreateKind::File));
                if sender.send(Ok(event)).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let (reported, reports) = mpsc::channel();
        std::thread::spawn(move || {
            report_quiet_changes(&receiver, quiet, move || {
                let _ = reported.send(());
            })
        });
        assert!(
            reports
                .recv_timeout(quiet * LONGEST_WAIT_IN_QUIET_PERIODS * 2)
                .is_ok(),
            "reported while the changes went on"
        );
        busy.join().unwrap();
    }
}
