//! Installing a newer release in place of the running program, on Windows and Linux
//! (docs/specs/2026-11-19-in-app-self-update.md).
//!
//! [`plan`] says whether this copy can update itself, and what it would download: the
//! release's archive for this platform, `SHA256SUMS.txt` and its minisign signature.
//! [`start_install`] downloads them on a worker thread, checks the signature with the
//! public key compiled in from `packaging/update-signing.pub` and the archive's
//! hash, unpacks the program and puts it in place of the running one, keeping a copy
//! of the old program beside it. [`HandOver::run`] then starts the new program, once
//! the old one has closed its window, and waits for it to confirm it is up; if it
//! doesn't, the old program is put back and started again.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

use crate::musicbrainz::USER_AGENT;
use crate::updates::{Asset, Release};

/// The public half of the key that signs releases. Until the key is made the file
/// holds only a comment, and no copy installs updates.
const PUBLIC_KEY: &str = include_str!("../../../packaging/update-signing.pub");
/// The release's checksums and their signature, as the release workflow names them.
const CHECKSUMS: &str = "SHA256SUMS.txt";
const SIGNATURE: &str = "SHA256SUMS.txt.minisig";
/// The licence texts that sit next to the program in the archive.
const LICENCES: [&str; 3] = ["LICENSE-MIT", "LICENSE-APACHE", "Sora-OFL.txt"];
/// The most an archive may weigh; releases are about 20 MB.
const ARCHIVE_LIMIT: u64 = 512 * 1024 * 1024;
const SMALL_FILE_LIMIT: u64 = 1024 * 1024;

/// Given to the new program: where to confirm that its window is up.
pub const STARTED_ARG: &str = "--update-started";
/// Given to the old program when it is started again: the version that failed.
pub const FAILED_ARG: &str = "--update-failed";
/// How long the old program waits for the new one to confirm.
pub const HAND_OVER_TIMEOUT: Duration = Duration::from_secs(60);

/// A system VCMP publishes an archive for and can update itself on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    WindowsX64,
    LinuxX64,
}

impl Platform {
    /// The system this program was built for, if it updates itself there.
    pub fn current() -> Option<Self> {
        if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Self::WindowsX64)
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Self::LinuxX64)
        } else {
            None
        }
    }

    /// The release's archive for this system: "vcmp-1.2.0-windows-x64.zip".
    pub fn archive(self, version: &str) -> String {
        match self {
            Self::WindowsX64 => format!("vcmp-{version}-windows-x64.zip"),
            Self::LinuxX64 => format!("vcmp-{version}-linux-x64.tar.gz"),
        }
    }

    /// The program's name in the archive.
    pub fn program(self) -> &'static str {
        match self {
            Self::WindowsX64 => "vcmp.exe",
            Self::LinuxX64 => "vcmp",
        }
    }
}

/// Why this copy only links to a newer release instead of installing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// Not Windows or Linux on x86-64.
    Platform,
    /// A debug build, which never replaces itself.
    DebugBuild,
    /// This build carries no key to check the signature with.
    NoKey,
    /// The release has no archive for this system, or no signed checksums.
    NotSigned,
    /// The folder the program runs from can't be written to.
    ReadOnly,
}

/// What installing a release takes: its files, and the program to replace.
#[derive(Clone, Debug)]
pub struct Plan {
    pub version: String,
    platform: Platform,
    program: PathBuf,
    archive: Asset,
    checksums: Asset,
    signature: Asset,
    key: PublicKey,
}

/// Whether this copy, running from `program`, can install `release` itself.
pub fn plan(release: &Release, program: &Path) -> Result<Plan, Blocker> {
    plan_with(
        release,
        program,
        Platform::current(),
        cfg!(debug_assertions),
        public_key(),
    )
}

fn plan_with(
    release: &Release,
    program: &Path,
    platform: Option<Platform>,
    debug: bool,
    key: Option<PublicKey>,
) -> Result<Plan, Blocker> {
    let platform = platform.ok_or(Blocker::Platform)?;
    if debug {
        return Err(Blocker::DebugBuild);
    }
    let key = key.ok_or(Blocker::NoKey)?;
    let asset = |name: &str| release.asset(name).cloned().ok_or(Blocker::NotSigned);
    let archive = asset(&platform.archive(&release.version))?;
    let checksums = asset(CHECKSUMS)?;
    let signature = asset(SIGNATURE)?;
    let folder = program.parent().ok_or(Blocker::ReadOnly)?;
    if !writable(folder) {
        return Err(Blocker::ReadOnly);
    }
    Ok(Plan {
        version: release.version.clone(),
        platform,
        program: program.to_path_buf(),
        archive,
        checksums,
        signature,
        key,
    })
}

/// The key compiled into this build, unless there is none yet.
fn public_key() -> Option<PublicKey> {
    PublicKey::decode(PUBLIC_KEY).ok()
}

/// Whether a file can be made in `folder`.
fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".vcmp-update-probe-{}", std::process::id()));
    match fs::File::create_new(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Why an install stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The user cancelled the download.
    Cancelled,
    /// A file could not be downloaded.
    Download(String),
    /// The download is not what VCMP's key signed.
    Verify(String),
    /// The archive could not be unpacked or the program not replaced.
    Install(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::Download(error) | Self::Verify(error) | Self::Install(error) => {
                write!(f, "{error}")
            }
        }
    }
}

/// Installs `plan` on a worker thread: reports the download's progress (0 to 1) to
/// `on_progress` and the result to `on_done`, both on that thread. Setting the
/// returned flag cancels the download.
pub fn start_install(
    plan: Plan,
    on_progress: impl Fn(f32) + Send + 'static,
    on_done: impl FnOnce(Result<HandOver, Failure>) + Send + 'static,
) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancelled = cancel.clone();
    thread::Builder::new()
        .name("vcmp-update-install".into())
        .spawn(move || {
            let result = install(&plan, &cancelled, &on_progress, |staged| {
                self_replace::self_replace(staged)
            });
            match &result {
                Ok(_) => log::info!("VCMP {} is in place", plan.version),
                Err(error) => log::warn!("could not install VCMP {}: {error}", plan.version),
            }
            on_done(result);
        })
        .expect("spawning the update install thread");
    cancel
}

/// Downloads, checks and puts the new program in place with `replace`, which swaps
/// the running program for the file it is given.
fn install(
    plan: &Plan,
    cancel: &AtomicBool,
    on_progress: &dyn Fn(f32),
    replace: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<HandOver, Failure> {
    let never = |_: u64| {};
    let signature = download(&plan.signature.url, SMALL_FILE_LIMIT, cancel, &never)?;
    let checksums = download(&plan.checksums.url, SMALL_FILE_LIMIT, cancel, &never)?;
    let total = plan.archive.size.max(1) as f32;
    let archive = download(&plan.archive.url, ARCHIVE_LIMIT, cancel, &|done| {
        on_progress((done as f32 / total).min(1.0))
    })?;
    verify(
        &plan.archive.name,
        &archive,
        &checksums,
        &signature,
        &plan.key,
    )
    .map_err(Failure::Verify)?;
    let files = extract(plan.platform, &archive).map_err(Failure::Install)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled);
    }
    place(&plan.program, plan.platform, &files, replace).map_err(Failure::Install)?;
    Ok(HandOver {
        program: plan.program.clone(),
        backup: backup_path(&plan.program),
        version: plan.version.clone(),
    })
}

/// One GET, read in pieces so that it can be cancelled and its progress shown.
fn download(
    url: &str,
    limit: u64,
    cancel: &AtomicBool,
    on_read: &dyn Fn(u64),
) -> Result<Vec<u8>, Failure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| Failure::Download(error.to_string()))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(Failure::Download(format!(
            "GitHub answered {status} for {url}"
        )));
    }
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let mut data = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| Failure::Download(error.to_string()))?;
        if read == 0 {
            return Ok(data);
        }
        data.extend_from_slice(&buffer[..read]);
        on_read(data.len() as u64);
    }
}

/// Checks that `checksums` carries `key`'s `signature` and lists `archive` under
/// `name` with its SHA-256.
fn verify(
    name: &str,
    archive: &[u8],
    checksums: &[u8],
    signature: &[u8],
    key: &PublicKey,
) -> Result<(), String> {
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|text| Signature::decode(text).ok())
        .ok_or("the signature file is not a minisign signature")?;
    key.verify(checksums, &signature, false)
        .map_err(|_| "SHA256SUMS.txt is not signed with VCMP's key".to_owned())?;
    let listed = std::str::from_utf8(checksums)
        .map_err(|_| "SHA256SUMS.txt is not text".to_owned())?
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let file = parts.next()?.trim_start_matches('*');
            (file == name).then(|| hash.to_ascii_lowercase())
        })
        .next()
        .ok_or_else(|| format!("SHA256SUMS.txt doesn't list {name}"))?;
    let actual: String = Sha256::digest(archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if listed == actual {
        Ok(())
    } else {
        Err(format!("{name} doesn't match its checksum"))
    }
}

/// The program and the licence texts in a release's archive, by file name.
fn extract(platform: Platform, archive: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let wanted = |name: &str| name == platform.program() || LICENCES.contains(&name);
    let mut files = Vec::new();
    match platform {
        Platform::WindowsX64 => {
            let mut zip = zip::ZipArchive::new(Cursor::new(archive))
                .map_err(|error| format!("the archive is no zip: {error}"))?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
                let name = file_name(&entry.name().map_err(|error| error.to_string())?);
                if entry.is_file() && wanted(&name) {
                    let mut data = Vec::new();
                    entry
                        .read_to_end(&mut data)
                        .map_err(|error| error.to_string())?;
                    files.push((name, data));
                }
            }
        }
        Platform::LinuxX64 => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
            let entries = tar
                .entries()
                .map_err(|error| format!("the archive is no tar.gz: {error}"))?;
            for entry in entries {
                let mut entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path().map_err(|error| error.to_string())?;
                let name = file_name(&path.to_string_lossy());
                if entry.header().entry_type().is_file() && wanted(&name) {
                    let mut data = Vec::new();
                    entry
                        .read_to_end(&mut data)
                        .map_err(|error| error.to_string())?;
                    files.push((name, data));
                }
            }
        }
    }
    if files.iter().any(|(name, _)| name == platform.program()) {
        Ok(files)
    } else {
        Err(format!("the archive holds no {}", platform.program()))
    }
}

/// The last part of a path in an archive, whichever separator it uses.
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

/// "vcmp.old.exe" beside "vcmp.exe", "vcmp.old" beside "vcmp".
fn backup_path(program: &Path) -> PathBuf {
    let stem = program.file_stem().unwrap_or_default().to_string_lossy();
    let name = match program.extension() {
        Some(extension) => format!("{stem}.old.{}", extension.to_string_lossy()),
        None => format!("{stem}.old"),
    };
    program.with_file_name(name)
}

/// Copies `program` to its backup, writes the new program beside it and has
/// `replace` swap it in, then refreshes the licence texts. If the swap fails the old
/// program stays and the backup goes.
fn place(
    program: &Path,
    platform: Platform,
    files: &[(String, Vec<u8>)],
    replace: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), String> {
    let folder = program.parent().ok_or("the program has no folder")?;
    let backup = backup_path(program);
    let staged = folder.join(format!(".vcmp-update-{}", platform.program()));
    let new_program = files
        .iter()
        .find(|(name, _)| name == platform.program())
        .map(|(_, data)| data)
        .ok_or("no program to install")?;
    fs::copy(program, &backup)
        .map_err(|error| format!("could not keep the old program: {error}"))?;
    let swapped = write_program(&staged, new_program).and_then(|()| replace(&staged));
    let _ = fs::remove_file(&staged);
    if let Err(error) = swapped {
        let _ = fs::remove_file(&backup);
        return Err(format!("could not replace the program: {error}"));
    }
    for (name, data) in files.iter().filter(|(name, _)| name != platform.program()) {
        if let Err(error) = fs::write(folder.join(name), data) {
            log::warn!("could not update {name}: {error}");
        }
    }
    Ok(())
}

fn write_program(path: &Path, data: &[u8]) -> io::Result<()> {
    fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// The new program in place, waiting to be started by the old one once it has
/// closed its window ([`HandOver::run`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandOver {
    /// The program, now the new version.
    pub program: PathBuf,
    /// The old program, kept until the new one is up.
    pub backup: PathBuf,
    /// The new version.
    pub version: String,
}

/// How a hand-over ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The new version confirmed it is up.
    Started,
    /// It didn't; the old program is back and was started again.
    RolledBack,
}

/// A started program, as the hand-over watches it.
pub trait Child {
    /// It has ended.
    fn exited(&mut self) -> bool;
    /// Ends it.
    fn end(&mut self);
}

impl Child for std::process::Child {
    fn exited(&mut self) -> bool {
        !matches!(self.try_wait(), Ok(None))
    }

    fn end(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

impl HandOver {
    /// Where the new program confirms that its window is up.
    pub fn marker(&self) -> PathBuf {
        self.program.with_file_name(".vcmp-update-started")
    }

    /// Starts the new program and waits for it, restoring the old one when it doesn't
    /// come up within [`HAND_OVER_TIMEOUT`].
    pub fn run(&self) -> Outcome {
        self.run_with(
            |program, args| {
                Command::new(program)
                    .args(args)
                    .spawn()
                    .map(|child| Box::new(child) as Box<dyn Child>)
            },
            HAND_OVER_TIMEOUT,
            Duration::from_millis(200),
        )
    }

    pub fn run_with(
        &self,
        mut start: impl FnMut(&Path, &[OsString]) -> io::Result<Box<dyn Child>>,
        timeout: Duration,
        poll: Duration,
    ) -> Outcome {
        let marker = self.marker();
        let _ = fs::remove_file(&marker);
        let args = [OsString::from(STARTED_ARG), marker.clone().into_os_string()];
        let started = match start(&self.program, &args) {
            Ok(mut child) => {
                let confirmed = wait_for(&marker, child.as_mut(), timeout, poll);
                if !confirmed {
                    child.end();
                }
                confirmed
            }
            Err(error) => {
                log::error!("could not start VCMP {}: {error}", self.version);
                false
            }
        };
        let _ = fs::remove_file(&marker);
        if started {
            let _ = fs::remove_file(&self.backup);
            return Outcome::Started;
        }
        log::error!(
            "VCMP {} did not start; putting the old program back",
            self.version
        );
        if let Err(error) = self.restore() {
            log::error!("could not put the old program back: {error}");
        }
        let args = [OsString::from(FAILED_ARG), OsString::from(&self.version)];
        if let Err(error) = start(&self.program, &args) {
            log::error!("could not start VCMP again: {error}");
        }
        Outcome::RolledBack
    }

    /// Moves the backup over the new program, retrying while Windows still holds the
    /// file of the program that just ended.
    fn restore(&self) -> io::Result<()> {
        let mut attempt = 0;
        loop {
            match fs::rename(&self.backup, &self.program) {
                Ok(()) => return Ok(()),
                Err(error) if attempt >= 20 => return Err(error),
                Err(_) => {
                    attempt += 1;
                    thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }
}

/// Waits until `marker` exists, `child` ends or `timeout` passes.
fn wait_for(marker: &Path, child: &mut dyn Child, timeout: Duration, poll: Duration) -> bool {
    let start = Instant::now();
    loop {
        if marker.exists() {
            return true;
        }
        if child.exited() || start.elapsed() >= timeout {
            return marker.exists();
        }
        thread::sleep(poll);
    }
}

/// Called by the new program once its window is up: the old one may go.
/// The marker is only ever made, never written into: the old program removed any
/// before it started this one, so an existing file at that path is someone else's.
pub fn confirm_started(marker: &Path) {
    if let Err(error) = fs::File::create_new(marker) {
        log::warn!("could not confirm the update: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Write;
    use std::rc::Rc;

    use super::*;

    struct Keys {
        public: PublicKey,
        secret: minisign::SecretKey,
    }

    fn keys() -> Keys {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = PublicKey::decode(&pair.pk.to_box().unwrap().to_string()).unwrap();
        Keys {
            public,
            secret: pair.sk,
        }
    }

    fn sign(keys: &Keys, data: &[u8]) -> Vec<u8> {
        minisign::sign(None, &keys.secret, Cursor::new(data), None, None)
            .unwrap()
            .into_string()
            .into_bytes()
    }

    fn sums(name: &str, data: &[u8]) -> Vec<u8> {
        let hash: String = Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("{hash}  other.zip\n{hash}  {name}\n").into_bytes()
    }

    fn release(names: &[&str]) -> Release {
        Release {
            version: "1.4.0".into(),
            url: "https://github.com/dalibor-osu/vcmp/releases/tag/v1.4.0".into(),
            prerelease: false,
            assets: names
                .iter()
                .map(|name| Asset {
                    name: (*name).into(),
                    url: format!("https://example.invalid/{name}"),
                    size: 10,
                })
                .collect(),
        }
    }

    const SIGNED: [&str; 4] = [
        "vcmp-1.4.0-windows-x64.zip",
        "vcmp-1.4.0-linux-x64.tar.gz",
        "SHA256SUMS.txt",
        "SHA256SUMS.txt.minisig",
    ];

    #[test]
    fn a_signed_release_can_be_installed_from_a_writable_folder() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("vcmp.exe");
        let plan = plan_with(
            &release(&SIGNED),
            &program,
            Some(Platform::WindowsX64),
            false,
            Some(keys().public),
        )
        .unwrap();
        assert_eq!(plan.archive.name, "vcmp-1.4.0-windows-x64.zip");
        assert_eq!(plan.version, "1.4.0");
        let plan = plan_with(
            &release(&SIGNED),
            &dir.path().join("vcmp"),
            Some(Platform::LinuxX64),
            false,
            Some(keys().public),
        )
        .unwrap();
        assert_eq!(plan.archive.name, "vcmp-1.4.0-linux-x64.tar.gz");
    }

    #[test]
    fn each_blocker_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("vcmp");
        let key = || Some(keys().public);
        let blocker = |release: &Release, platform, debug, key, program: &Path| {
            plan_with(release, program, platform, debug, key).unwrap_err()
        };
        let signed = release(&SIGNED);
        let linux = Some(Platform::LinuxX64);
        assert_eq!(
            blocker(&signed, None, false, key(), &program),
            Blocker::Platform
        );
        assert_eq!(
            blocker(&signed, linux, true, key(), &program),
            Blocker::DebugBuild
        );
        assert_eq!(
            blocker(&signed, linux, false, None, &program),
            Blocker::NoKey
        );
        for missing in SIGNED.iter().filter(|name| !name.contains("windows")) {
            let names: Vec<&str> = SIGNED
                .iter()
                .copied()
                .filter(|name| name != missing)
                .collect();
            assert_eq!(
                blocker(&release(&names), linux, false, key(), &program),
                Blocker::NotSigned,
                "without {missing}"
            );
        }
        let gone = dir.path().join("no such folder").join("vcmp");
        assert_eq!(
            blocker(&signed, linux, false, key(), &gone),
            Blocker::ReadOnly
        );
    }

    #[test]
    fn the_placeholder_key_file_holds_no_key_or_a_real_one() {
        let first = PUBLIC_KEY.lines().next().unwrap_or_default();
        assert!(first.starts_with("untrusted comment:"), "{first:?}");
        assert_eq!(public_key().is_some(), PUBLIC_KEY.lines().count() >= 2);
    }

    #[test]
    fn a_signed_archive_with_its_checksum_verifies() {
        let keys = keys();
        let archive = b"the archive";
        let checksums = sums("vcmp-1.4.0-linux-x64.tar.gz", archive);
        let signature = sign(&keys, &checksums);
        assert_eq!(
            verify(
                "vcmp-1.4.0-linux-x64.tar.gz",
                archive,
                &checksums,
                &signature,
                &keys.public
            ),
            Ok(())
        );
    }

    #[test]
    fn tampering_is_caught() {
        let keys = keys();
        let name = "vcmp-1.4.0-linux-x64.tar.gz";
        let archive = b"the archive";
        let checksums = sums(name, archive);
        let signature = sign(&keys, &checksums);

        assert!(
            verify(
                name,
                b"another archive",
                &checksums,
                &signature,
                &keys.public
            )
            .is_err()
        );
        let forged = sums(name, b"another archive");
        assert!(verify(name, b"another archive", &forged, &signature, &keys.public).is_err());
        let other_key = self::keys();
        let resigned = sign(&other_key, &forged);
        assert!(verify(name, b"another archive", &forged, &resigned, &keys.public).is_err());
        assert!(
            verify(
                "vcmp-9.9.9-linux-x64.tar.gz",
                archive,
                &checksums,
                &signature,
                &keys.public
            )
            .is_err()
        );
        assert!(verify(name, archive, &checksums, b"not a signature", &keys.public).is_err());
    }

    fn zip_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    fn tar_gz_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(encoder);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn the_program_and_licences_come_out_of_either_archive() {
        let zip = zip_archive(&[
            ("vcmp-1.4.0-windows-x64/vcmp.exe", b"new program"),
            ("vcmp-1.4.0-windows-x64\\LICENSE-MIT", b"mit"),
            ("vcmp-1.4.0-windows-x64/README.txt", b"ignored"),
        ]);
        let files = extract(Platform::WindowsX64, &zip).unwrap();
        assert_eq!(
            files,
            [
                ("vcmp.exe".to_owned(), b"new program".to_vec()),
                ("LICENSE-MIT".to_owned(), b"mit".to_vec())
            ]
        );

        let tar = tar_gz_archive(&[
            ("vcmp-1.4.0-linux-x64/vcmp", b"new program"),
            ("vcmp-1.4.0-linux-x64/Sora-OFL.txt", b"ofl"),
        ]);
        let files = extract(Platform::LinuxX64, &tar).unwrap();
        assert_eq!(files[0], ("vcmp".to_owned(), b"new program".to_vec()));
        assert_eq!(files[1], ("Sora-OFL.txt".to_owned(), b"ofl".to_vec()));
    }

    #[test]
    fn an_archive_without_the_program_is_refused() {
        let zip = zip_archive(&[("vcmp-1.4.0-windows-x64/LICENSE-MIT", b"mit")]);
        assert!(extract(Platform::WindowsX64, &zip).is_err());
        assert!(extract(Platform::LinuxX64, b"not an archive").is_err());
        assert!(extract(Platform::WindowsX64, b"not an archive").is_err());
    }

    /// The swap self_replace makes, on files that aren't running.
    fn copy_into(program: &Path) -> impl FnOnce(&Path) -> io::Result<()> + '_ {
        move |staged| fs::copy(staged, program).map(|_| ())
    }

    #[test]
    fn placing_keeps_the_old_program_and_refreshes_the_licences() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("vcmp.exe");
        fs::write(&program, b"old program").unwrap();
        fs::write(dir.path().join("LICENSE-MIT"), b"old mit").unwrap();
        let files = vec![
            ("vcmp.exe".to_owned(), b"new program".to_vec()),
            ("LICENSE-MIT".to_owned(), b"new mit".to_vec()),
        ];
        place(&program, Platform::WindowsX64, &files, copy_into(&program)).unwrap();
        assert_eq!(fs::read(&program).unwrap(), b"new program");
        assert_eq!(
            fs::read(dir.path().join("vcmp.old.exe")).unwrap(),
            b"old program"
        );
        assert_eq!(
            fs::read(dir.path().join("LICENSE-MIT")).unwrap(),
            b"new mit"
        );
        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["LICENSE-MIT", "vcmp.exe", "vcmp.old.exe"]);
    }

    #[test]
    fn a_failed_swap_leaves_the_old_program_alone() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("vcmp");
        fs::write(&program, b"old program").unwrap();
        let files = vec![("vcmp".to_owned(), b"new program".to_vec())];
        let error = place(&program, Platform::LinuxX64, &files, |_| {
            Err(io::Error::other("locked"))
        })
        .unwrap_err();
        assert!(error.contains("locked"), "{error}");
        assert_eq!(fs::read(&program).unwrap(), b"old program");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no backup or staged file left"
        );
    }

    #[test]
    fn backups_sit_beside_the_program() {
        assert_eq!(
            backup_path(Path::new("/a/vcmp.exe")),
            Path::new("/a/vcmp.old.exe")
        );
        assert_eq!(backup_path(Path::new("/a/vcmp")), Path::new("/a/vcmp.old"));
    }

    /// A child that, after `ticks` polls, confirms (when `confirms`) or ends.
    struct FakeChild {
        marker: Option<PathBuf>,
        ticks: u32,
        ended: Rc<RefCell<bool>>,
    }

    impl Child for FakeChild {
        fn exited(&mut self) -> bool {
            if self.ticks > 0 {
                self.ticks -= 1;
                return false;
            }
            match &self.marker {
                Some(marker) => {
                    fs::write(marker, b"").unwrap();
                    false
                }
                None => true,
            }
        }

        fn end(&mut self) {
            *self.ended.borrow_mut() = true;
        }
    }

    fn hand_over(dir: &Path) -> HandOver {
        let program = dir.join("vcmp");
        fs::write(&program, b"new program").unwrap();
        fs::write(dir.join("vcmp.old"), b"old program").unwrap();
        HandOver {
            program,
            backup: dir.join("vcmp.old"),
            version: "1.4.0".into(),
        }
    }

    type Starts = Rc<RefCell<Vec<Vec<OsString>>>>;

    fn run(hand_over: &HandOver, confirms: bool, timeout: Duration) -> (Outcome, Starts, bool) {
        let starts: Starts = Rc::default();
        let ended = Rc::new(RefCell::new(false));
        let outcome = hand_over.run_with(
            |program, args| {
                assert_eq!(program, hand_over.program);
                starts.borrow_mut().push(args.to_vec());
                Ok(Box::new(FakeChild {
                    marker: (confirms && args[0] == STARTED_ARG).then(|| PathBuf::from(&args[1])),
                    ticks: 2,
                    ended: ended.clone(),
                }) as Box<dyn Child>)
            },
            timeout,
            Duration::from_millis(1),
        );
        let ended = *ended.borrow();
        (outcome, starts, ended)
    }

    #[test]
    fn a_new_version_that_confirms_keeps_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let hand_over = hand_over(dir.path());
        let (outcome, starts, ended) = run(&hand_over, true, Duration::from_secs(5));
        assert_eq!(outcome, Outcome::Started);
        assert!(!ended);
        assert_eq!(starts.borrow().len(), 1);
        assert_eq!(starts.borrow()[0][0], STARTED_ARG);
        assert_eq!(fs::read(&hand_over.program).unwrap(), b"new program");
        assert!(!hand_over.backup.exists(), "the old program goes");
        assert!(!hand_over.marker().exists());
    }

    #[test]
    fn a_new_version_that_ends_is_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let hand_over = hand_over(dir.path());
        let (outcome, starts, _) = run(&hand_over, false, Duration::from_secs(5));
        assert_eq!(outcome, Outcome::RolledBack);
        assert_eq!(fs::read(&hand_over.program).unwrap(), b"old program");
        assert!(!hand_over.backup.exists());
        let starts = starts.borrow();
        assert_eq!(starts.len(), 2, "the old program is started again");
        assert_eq!(
            starts[1],
            [OsString::from(FAILED_ARG), OsString::from("1.4.0")]
        );
    }

    #[test]
    fn a_new_version_that_hangs_is_ended_and_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let hand_over = hand_over(dir.path());
        let starts: Starts = Rc::default();
        let ended = Rc::new(RefCell::new(false));
        let outcome = hand_over.run_with(
            |_, args| {
                starts.borrow_mut().push(args.to_vec());
                Ok(Box::new(FakeChild {
                    marker: None,
                    ticks: u32::MAX,
                    ended: ended.clone(),
                }) as Box<dyn Child>)
            },
            Duration::from_millis(20),
            Duration::from_millis(1),
        );
        assert_eq!(outcome, Outcome::RolledBack);
        assert!(*ended.borrow(), "the hanging program is ended");
        assert_eq!(fs::read(&hand_over.program).unwrap(), b"old program");
    }

    #[test]
    fn a_new_version_that_cannot_start_is_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let hand_over = hand_over(dir.path());
        let mut calls = 0;
        let outcome = hand_over.run_with(
            |_, _| {
                calls += 1;
                Err(io::Error::other("not an executable"))
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
        );
        assert_eq!(outcome, Outcome::RolledBack);
        assert_eq!(calls, 2);
        assert_eq!(fs::read(&hand_over.program).unwrap(), b"old program");
    }
}
