//! Asks MusicBrainz and the Cover Art Archive for the covers the picker would offer
//! for one album, prints them and saves their previews and the first one at full
//! size under `target/cover-candidates/`. It sends the artist and title given to
//! musicbrainz.org, at the pace the app keeps.
//!
//!     cargo run -p vcmp-integrations --example cover_candidates -- "Artist" "Album" [tracks]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use vcmp_integrations::musicbrainz::{AlbumQuery, Cover, Searcher};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(artist), Some(title)) = (args.next(), args.next()) else {
        eprintln!("usage: cover_candidates <artist> <album> [track count]");
        std::process::exit(2);
    };
    let track_count = args.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/cover-candidates");
    std::fs::create_dir_all(&out).expect("output directory");

    let album = AlbumQuery {
        artist,
        title,
        track_count,
    };
    let mut searcher = Searcher::online();
    let mut first = None;
    let result = searcher.candidates(&album, None, &AtomicBool::new(false), |candidate| {
        let (kind, id) = match &candidate.cover {
            Cover::Release(id) => ("release", id),
            Cover::ReleaseGroup(id) => ("release-group", id),
        };
        let path = out.join(format!("{kind}-{id}.jpg"));
        std::fs::write(&path, &candidate.preview).expect("preview written");
        println!(
            "{kind} {id}: {:?}, {:?}, {:?}, {} bytes",
            candidate.title,
            candidate.year,
            candidate.country,
            candidate.preview.len()
        );
        first.get_or_insert(candidate.cover);
    });
    println!("search: {result:?}");
    if let Some(cover) = first {
        match searcher.download(&cover) {
            Ok(image) => {
                let path = out.join("picked.jpg");
                std::fs::write(&path, &image).expect("image written");
                println!("downloaded {} bytes to {}", image.len(), path.display());
            }
            Err(error) => println!("download failed: {error}"),
        }
    }
}
