use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use vcmp_library::{Library, RemoteFile, ServerId, parse_remote_key};
use vcmp_playback::{LocalSource, PlaybackError, TrackSource, TrackStream};
use vcmp_protocol::audio_etag;

use crate::Connection;

pub const DEFAULT_CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

const QUEUE_MAX: usize = 4;

const RECENTLY_USED: Duration = Duration::from_secs(15 * 60);

const RETRY_FAILED: Duration = Duration::from_secs(30);

enum State {
    Queued,
    Downloading,
    Failed(String, Instant),
}

#[derive(Default)]
struct Jobs {
    queue: VecDeque<String>,
    states: HashMap<String, State>,
}

struct Inner {
    folder: PathBuf,
    limit: AtomicU64,
    library: Mutex<Library>,
    connections: RwLock<HashMap<ServerId, Connection>>,
    jobs: Mutex<Jobs>,
    wake: Condvar,
}

#[derive(Clone)]
pub struct ServerSource {
    inner: Arc<Inner>,
}

impl ServerSource {
    pub fn new(folder: PathBuf, library: Library, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(&folder)?;
        let inner = Arc::new(Inner {
            folder,
            limit: AtomicU64::new(limit),
            library: Mutex::new(library),
            connections: RwLock::default(),
            jobs: Mutex::default(),
            wake: Condvar::new(),
        });
        let worker = inner.clone();
        std::thread::Builder::new()
            .name("vcmp-server-downloads".into())
            .spawn(move || worker.run())?;
        Ok(Self { inner })
    }

    pub fn set_connection(&self, server: ServerId, connection: Option<Connection>) {
        let mut connections = self
            .inner
            .connections
            .write()
            .unwrap_or_else(|e| e.into_inner());
        match connection {
            Some(connection) => connections.insert(server, connection),
            None => connections.remove(&server),
        };
    }

    pub fn set_limit(&self, bytes: u64) {
        self.inner.limit.store(bytes, Ordering::Relaxed);
        self.inner.evict(None);
    }

    pub fn size(&self) -> u64 {
        self.inner.files().iter().map(|(_, size, _)| size).sum()
    }

    pub fn clear(&self) {
        for (path, _, _) in self.inner.files() {
            let _ = fs::remove_file(path);
        }
    }

    pub fn is_cached(&self, key: &str) -> bool {
        self.inner
            .resolve(key)
            .ok()
            .is_some_and(|file| self.inner.cached(&file).is_some())
    }
}

impl TrackSource for ServerSource {
    fn open(&self, id: &str) -> Result<Box<dyn TrackStream>, PlaybackError> {
        let file = self.inner.resolve(id)?;
        if let Some(path) = self.inner.cached(&file) {
            touch(&path);
            let text = path.to_str().ok_or_else(|| {
                PlaybackError::Io(io::ErrorKind::InvalidData, "cache path".into())
            })?;
            return LocalSource.open(text);
        }
        let mut jobs = self.inner.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(State::Failed(_, at)) = jobs.states.get(id)
            && at.elapsed() >= RETRY_FAILED
        {
            jobs.states.remove(id);
        }
        match jobs.states.get(id) {
            Some(State::Failed(message, _)) => {
                let message = message.clone();
                jobs.states.remove(id);
                Err(PlaybackError::Io(io::ErrorKind::Other, message))
            }
            Some(State::Downloading) => Err(PlaybackError::NotReady),
            Some(State::Queued) => {
                jobs.queue.retain(|queued| queued != id);
                jobs.queue.push_front(id.to_owned());
                Err(PlaybackError::NotReady)
            }
            None => {
                jobs.states.insert(id.to_owned(), State::Queued);
                jobs.queue.push_front(id.to_owned());
                trim(&mut jobs);
                self.inner.wake.notify_one();
                Err(PlaybackError::NotReady)
            }
        }
    }

    fn prepare(&self, id: &str) {
        let Ok(file) = self.inner.resolve(id) else {
            return;
        };
        if self.inner.cached(&file).is_some() {
            return;
        }
        let mut jobs = self.inner.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let stale = matches!(jobs.states.get(id), Some(State::Failed(_, at)) if at.elapsed() >= RETRY_FAILED);
        if stale {
            jobs.states.remove(id);
        }
        if jobs.states.contains_key(id) {
            return;
        }
        jobs.states.insert(id.to_owned(), State::Queued);
        jobs.queue.push_back(id.to_owned());
        trim(&mut jobs);
        self.inner.wake.notify_one();
    }
}

fn trim(jobs: &mut Jobs) {
    while jobs.queue.len() > QUEUE_MAX {
        if let Some(dropped) = jobs.queue.pop_back() {
            jobs.states.remove(&dropped);
        }
    }
}

fn touch(path: &Path) {
    if let Ok(file) = OpenOptions::new().append(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn extension(file: &RemoteFile) -> &'static str {
    use vcmp_library::Format;
    match file.format {
        Some(Format::Mp3) => "mp3",
        Some(Format::Flac) => "flac",
        Some(Format::OggVorbis) => "ogg",
        Some(Format::Wav) => "wav",
        Some(Format::Aiff) => "aiff",
        Some(Format::Aac | Format::Alac) => "m4a",
        _ => "bin",
    }
}

impl Inner {
    fn resolve(&self, key: &str) -> Result<RemoteFile, PlaybackError> {
        let (server, remote) = parse_remote_key(key).ok_or_else(|| {
            PlaybackError::Io(
                io::ErrorKind::InvalidInput,
                format!("{key} names no server track"),
            )
        })?;
        let library = self.library.lock().unwrap_or_else(|e| e.into_inner());
        let file = library
            .remote_track(server, remote)
            .and_then(|track| match track {
                Some(track) => library.remote_file(track),
                None => Ok(None),
            })
            .map_err(|error| PlaybackError::Io(io::ErrorKind::Other, error.to_string()))?;
        match file {
            Some(file) if file.available => Ok(file),
            _ => Err(PlaybackError::Io(
                io::ErrorKind::NotFound,
                "the server no longer has this track".into(),
            )),
        }
    }

    fn path(&self, file: &RemoteFile) -> PathBuf {
        self.folder.join(format!("{}", file.server.0)).join(format!(
            "{}-{}-{}.{}",
            file.remote_id,
            file.size,
            file.modified_ms,
            extension(file)
        ))
    }

    fn cached(&self, file: &RemoteFile) -> Option<PathBuf> {
        let path = self.path(file);
        fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() > 0)
            .map(|_| path)
    }

    fn run(&self) {
        loop {
            let key = {
                let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(key) = jobs.queue.pop_front() {
                        jobs.states.insert(key.clone(), State::Downloading);
                        break key;
                    }
                    jobs = self.wake.wait(jobs).unwrap_or_else(|e| e.into_inner());
                }
            };
            let result = self.fetch(&key);
            let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(()) => {
                    jobs.states.remove(&key);
                }
                Err(message) => {
                    log::warn!("{key} could not be downloaded: {message}");
                    jobs.states
                        .insert(key, State::Failed(message, Instant::now()));
                }
            }
        }
    }

    fn fetch(&self, key: &str) -> Result<(), String> {
        let file = self.resolve(key).map_err(|error| error.to_string())?;
        if self.cached(&file).is_some() {
            return Ok(());
        }
        let connection = self
            .connections
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&file.server)
            .cloned()
            .ok_or_else(|| "the track's server is not connected".to_owned())?;
        let path = self.path(&file);
        let partial = path.with_extension(format!("{}.part", extension(&file)));
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        let etag = audio_etag(file.size, file.modified_ms);
        let have = fs::metadata(&partial).map_or(0, |metadata| metadata.len());
        let resume = (have > 0 && have < file.size).then_some((have, etag.as_str()));
        let download = connection
            .download(file.remote_id, resume, &partial)
            .map_err(|error| error.to_string())?;
        if download.etag.as_deref().is_some_and(|given| given != etag) {
            log::info!("{key} changed on its server since the last sync");
        }
        fs::rename(&partial, &path).map_err(|error| error.to_string())?;
        self.evict(Some(&path));
        Ok(())
    }

    fn files(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let mut files = Vec::new();
        let Ok(servers) = fs::read_dir(&self.folder) else {
            return files;
        };
        for server in servers.flatten() {
            let Ok(entries) = fs::read_dir(server.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata()
                    && metadata.is_file()
                {
                    files.push((
                        entry.path(),
                        metadata.len(),
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    ));
                }
            }
        }
        files
    }

    fn evict(&self, keep: Option<&Path>) {
        let limit = self.limit.load(Ordering::Relaxed);
        let mut files = self.files();
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        if total <= limit {
            return;
        }
        files.sort_by_key(|(_, _, used)| *used);
        let recent = SystemTime::now() - RECENTLY_USED;
        for (path, size, used) in files {
            if total <= limit {
                break;
            }
            if Some(path.as_path()) == keep || used > recent {
                continue;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
            }
        }
    }
}
