//! Searches a library database the way the app's search page does, and prints what
//! it finds and how long each search took.
//!
//!     cargo run --release -p vcmp-library --example search -- target/scan-example "ライブ" "高坂"

use std::path::PathBuf;
use std::time::Instant;

use vcmp_library::Library;

fn main() {
    let mut args = std::env::args().skip(1);
    let home = PathBuf::from(args.next().expect("usage: search LIBRARY_DIR TEXT..."));
    let library =
        Library::open(&home.join("library.sqlite3"), &home.join("cache")).expect("library");
    for text in args {
        let started = Instant::now();
        let results = library.search(&text, 50).expect("search");
        let took = started.elapsed();
        println!(
            "{text:?}: {} artists, {} albums, {} tracks in {:.1} ms",
            results.artists.len(),
            results.albums.len(),
            results.tracks.len(),
            took.as_secs_f64() * 1000.0
        );
        for track in results.tracks.iter().take(3) {
            println!(
                "    {} - {}",
                track.artist.as_deref().unwrap_or("?"),
                track.title
            );
        }
    }
}
