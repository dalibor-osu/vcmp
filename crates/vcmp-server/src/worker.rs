use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::oneshot;
use vcmp_library::{Library, LibraryWatcher, SourceDir, TagEdit, TagWriteError, TrackId};
use vcmp_protocol::{
    ErrorCode, FailedTagWrite, ScanStatus, ScanSummary, SkippedFile, TagWriteFailure,
    TagWriteResult,
};

use crate::api::Failure;
use crate::tokens::Tokens;

const WATCH_QUIET_PERIOD: Duration = Duration::from_secs(5);

const READERS_KEPT: usize = 8;

pub(crate) enum Job {
    Scan,
    AddFolder {
        path: PathBuf,
        recursive: bool,
        reply: oneshot::Sender<Result<SourceDir, Failure>>,
    },
    RemoveFolder {
        id: i64,
        reply: oneshot::Sender<Result<(), Failure>>,
    },
    WriteTags {
        groups: Vec<(Vec<TrackId>, TagEdit)>,
        reply: oneshot::Sender<Result<TagWriteResult, Failure>>,
    },
    Stop,
}

pub(crate) struct State {
    pub server_id: String,
    pub name: String,
    pub tokens: Tokens,
    pub configured_music: Vec<PathBuf>,
    pub jobs: Sender<Job>,
    pub scan: Mutex<ScanStatus>,
    pub can_write: AtomicBool,
    pub cancel: AtomicBool,
    database: PathBuf,
    cache: PathBuf,
    readers: Mutex<Vec<Library>>,
}

impl State {
    pub fn new(
        server_id: String,
        name: String,
        tokens: Tokens,
        configured_music: Vec<PathBuf>,
        jobs: Sender<Job>,
        database: PathBuf,
        cache: PathBuf,
    ) -> Self {
        Self {
            server_id,
            name,
            tokens,
            configured_music,
            jobs,
            scan: Mutex::new(ScanStatus::default()),
            can_write: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            database,
            cache,
            readers: Mutex::new(Vec::new()),
        }
    }

    pub fn read<T>(&self, read: impl FnOnce(&Library) -> Result<T, Failure>) -> Result<T, Failure> {
        let idle = self.readers.lock().unwrap_or_else(|e| e.into_inner()).pop();
        let library = match idle {
            Some(library) => library,
            None => Library::open(&self.database, &self.cache).map_err(Failure::internal)?,
        };
        let result = read(&library);
        let mut readers = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        if readers.len() < READERS_KEPT {
            readers.push(library);
        }
        result
    }

    pub fn is_configured_folder(&self, path: &Path) -> bool {
        self.configured_music.iter().any(|folder| folder == path)
    }

    fn scan_status(&self) -> std::sync::MutexGuard<'_, ScanStatus> {
        self.scan.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) fn run(
    mut library: Library,
    state: Arc<State>,
    jobs: Receiver<Job>,
    rescan: Option<Duration>,
) {
    let mut watcher = None;
    let mut scan_due = true;
    let mut next_rescan = None;
    let mut folders_changed = true;
    loop {
        if folders_changed {
            folders_changed = false;
            state
                .can_write
                .store(folders_writable(&library), Ordering::Relaxed);
            drop(watcher.take());
            watcher = watch(&library, &state.jobs);
        }
        if scan_due {
            scan_due = false;
            scan(&mut library, &state);
            next_rescan = rescan.map(|interval| Instant::now() + interval);
            continue;
        }
        let wait = next_rescan
            .map(|at| at.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(24 * 60 * 60));
        match jobs.recv_timeout(wait) {
            Ok(Job::Scan) => scan_due = true,
            Ok(Job::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => scan_due = next_rescan.is_some(),
            Ok(Job::AddFolder {
                path,
                recursive,
                reply,
            }) => {
                let result = add_folder(&library, &path, recursive);
                if result.is_ok() {
                    folders_changed = true;
                    scan_due = true;
                }
                let _ = reply.send(result);
            }
            Ok(Job::RemoveFolder { id, reply }) => {
                let result = remove_folder(&library, &state, id);
                if result.is_ok() {
                    folders_changed = true;
                    scan_due = true;
                }
                let _ = reply.send(result);
            }
            Ok(Job::WriteTags { groups, reply }) => {
                let _ = reply.send(write_tags(&mut library, &state, &groups));
            }
        }
    }
    drop(watcher);
}

fn watch(library: &Library, jobs: &Sender<Job>) -> Option<LibraryWatcher> {
    let sources = match library.sources() {
        Ok(sources) => sources,
        Err(error) => {
            log::error!("the music folders could not be read: {error}");
            return None;
        }
    };
    if sources.is_empty() {
        return None;
    }
    let jobs = jobs.clone();
    match LibraryWatcher::start(&sources, WATCH_QUIET_PERIOD, move || {
        let _ = jobs.send(Job::Scan);
    }) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log::warn!("the music folders can't be watched, only rescanned: {error}");
            None
        }
    }
}

fn scan(library: &mut Library, state: &State) {
    state.scan_status().running = true;
    log::info!("scanning the music folders");
    let started = Instant::now();
    let result = library.scan(&state.cancel, |_| {});
    let summary = match result {
        Ok(report) => {
            log::info!(
                "scan finished in {:.1} s: {} added, {} updated, {} moved, {} gone, {} back, {} skipped",
                started.elapsed().as_secs_f64(),
                report.added,
                report.updated,
                report.relinked,
                report.became_unavailable,
                report.became_available,
                report.skipped.len()
            );
            for folder in &report.unreachable_sources {
                log::warn!("the music folder {} can't be read", folder.display());
            }
            ScanSummary {
                finished_ms: now_ms(),
                added: report.added,
                updated: report.updated,
                relinked: report.relinked,
                became_unavailable: report.became_unavailable,
                became_available: report.became_available,
                unchanged: report.unchanged,
                skipped: report
                    .skipped
                    .into_iter()
                    .map(|file| SkippedFile {
                        path: file.path.to_string_lossy().into_owned(),
                        reason: file.reason,
                    })
                    .collect(),
                unreachable_folders: report
                    .unreachable_sources
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
                error: None,
            }
        }
        Err(error) => {
            log::error!("the scan failed: {error}");
            ScanSummary {
                finished_ms: now_ms(),
                error: Some(error.to_string()),
                ..ScanSummary::default()
            }
        }
    };
    let mut status = state.scan_status();
    status.running = false;
    status.last = Some(summary);
}

fn add_folder(library: &Library, path: &Path, recursive: bool) -> Result<SourceDir, Failure> {
    library
        .add_source(path, recursive)
        .map_err(|error| match error {
            vcmp_library::Error::Io { .. } | vcmp_library::Error::NotADirectory(_) => Failure::new(
                ErrorCode::NotAFolder,
                format!("{} is not a folder the server can read", path.display()),
            ),
            error => Failure::internal(error),
        })
}

fn remove_folder(library: &Library, state: &State, id: i64) -> Result<(), Failure> {
    let sources = library.sources().map_err(Failure::internal)?;
    let Some(source) = sources.iter().find(|source| source.id == id) else {
        return Err(Failure::not_found("no such music folder"));
    };
    if state.is_configured_folder(&source.path) {
        return Err(Failure::new(
            ErrorCode::ConfiguredFolder,
            "the folder is named in the server's configuration; remove it there",
        ));
    }
    library.remove_source(id).map_err(Failure::internal)
}

fn write_tags(
    library: &mut Library,
    state: &State,
    groups: &[(Vec<TrackId>, TagEdit)],
) -> Result<TagWriteResult, Failure> {
    let edits: Vec<(TrackId, &TagEdit)> = groups
        .iter()
        .flat_map(|(tracks, edit)| tracks.iter().map(move |track| (*track, edit)))
        .collect();
    let report = library
        .write_tags(&edits, &state.cancel, |_, _| {})
        .map_err(|error| match error {
            vcmp_library::Error::InvalidTagEdit(invalid) => {
                Failure::new(ErrorCode::InvalidTagEdit, invalid.to_string())
            }
            error => Failure::internal(error),
        })?;
    Ok(TagWriteResult {
        written: report.written.iter().map(|track| track.0).collect(),
        failed: report
            .failed
            .into_iter()
            .map(|failed| FailedTagWrite {
                track: failed.track.0,
                reason: failure_reason(&failed.error),
                detail: failed.error.to_string(),
            })
            .collect(),
    })
}

fn failure_reason(error: &TagWriteError) -> TagWriteFailure {
    match error {
        TagWriteError::Unavailable => TagWriteFailure::Unavailable,
        TagWriteError::ReadOnly => TagWriteFailure::ReadOnly,
        TagWriteError::Unreadable(_) => TagWriteFailure::Unreadable,
        TagWriteError::Write(_) => TagWriteFailure::Write,
        TagWriteError::Verify(_) => TagWriteFailure::Verify,
        TagWriteError::ChangedMeanwhile => TagWriteFailure::ChangedMeanwhile,
        TagWriteError::Replace(_) => TagWriteFailure::Replace,
        TagWriteError::UnsupportedKey(_) => TagWriteFailure::UnsupportedKey,
        TagWriteError::Conflict(_) => TagWriteFailure::Conflict,
    }
}

fn folders_writable(library: &Library) -> bool {
    let Ok(sources) = library.sources() else {
        return false;
    };
    let reachable: Vec<&SourceDir> = sources.iter().filter(|s| s.path.is_dir()).collect();
    !reachable.is_empty()
        && reachable.iter().all(|source| {
            tempfile::Builder::new()
                .prefix(".vcmp-write-check")
                .tempfile_in(&source.path)
                .is_ok()
        })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
