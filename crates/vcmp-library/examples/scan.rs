//! Scans music directories into a throwaway database and prints what the library
//! made of them: counts, skipped files, artwork coverage and timings. Music files are
//! only read; the database and artwork cache go to `target/scan-example/`.
//!
//!     cargo run --release -p vcmp-library --example scan -- <music dir>...
//!
//! Add `--keep` to reuse the previous run's database and time an incremental rescan.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use vcmp_library::{Library, ScanPhase};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut keep = false;
    let mut dirs = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--keep" => keep = true,
            _ => dirs.push(PathBuf::from(arg)),
        }
    }
    if dirs.is_empty() {
        return Err("usage: scan [--keep] <music dir>...".into());
    }

    let work = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/scan-example");
    if !keep && work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    let mut library = Library::open(&work.join("library.sqlite3"), &work.join("cache"))?;
    for dir in &dirs {
        library.add_source(dir, true)?;
    }

    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut phase_started = Instant::now();
    let mut last_phase = ScanPhase::Discovering;
    let report = library.scan(&cancel, |progress| {
        if progress.phase != last_phase {
            eprintln!(
                "  {:?} took {:.1} s",
                last_phase,
                phase_started.elapsed().as_secs_f64()
            );
            last_phase = progress.phase;
            phase_started = Instant::now();
        }
    })?;
    eprintln!(
        "  {:?} took {:.1} s",
        last_phase,
        phase_started.elapsed().as_secs_f64()
    );
    println!("scan took {:.1} s", started.elapsed().as_secs_f64());
    println!(
        "added {}, updated {}, relinked {}, unchanged {}, unavailable {}, available again {}",
        report.added,
        report.updated,
        report.relinked,
        report.unchanged,
        report.became_unavailable,
        report.became_available
    );
    println!("unreachable sources: {:?}", report.unreachable_sources);
    println!("skipped {} files", report.skipped.len());
    for skipped in report.skipped.iter().take(15) {
        println!("  {}: {}", skipped.path.display(), skipped.reason);
    }

    let albums = library.albums()?;
    let with_art = albums
        .iter()
        .filter(|album| album.artwork.is_some())
        .count();
    let tracks = library.tracks()?;
    println!(
        "tracks {}, albums {} ({} with artwork), artists {}",
        tracks.len(),
        albums.len(),
        with_art,
        library.artists()?.len()
    );
    println!(
        "tracks without album: {}",
        tracks.iter().filter(|track| track.album.is_none()).count()
    );
    println!(
        "tracks without artist: {}",
        tracks.iter().filter(|track| track.artist.is_none()).count()
    );
    let long = tracks
        .iter()
        .filter(|track| track.duration.as_secs() > 25 * 60)
        .count();
    println!("tracks longer than 25 min (possible single-file album images): {long}");
    let single_track_albums = albums.iter().filter(|album| album.track_count == 1).count();
    println!("albums with a single track: {single_track_albums}");
    Ok(())
}
