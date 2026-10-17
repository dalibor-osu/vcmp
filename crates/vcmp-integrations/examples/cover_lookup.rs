//! Looks one album up on MusicBrainz the way the Discord presence does, and prints
//! the cover it would show. Sends the artist and title to musicbrainz.org.
//!
//!     cargo run -p vcmp-integrations --example cover_lookup -- "Artist" "Album" [tracks]

use vcmp_integrations::musicbrainz::{AlbumQuery, Lookup, Searcher};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(artist), Some(title)) = (args.next(), args.next()) else {
        eprintln!("usage: cover_lookup ARTIST ALBUM [TRACKS]");
        std::process::exit(2);
    };
    let track_count = args.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    let album = AlbumQuery {
        artist,
        title,
        track_count,
    };
    match Searcher::online().find(&album) {
        Lookup::Found(cover) => println!("found: {cover:?}\n{}", cover.url()),
        Lookup::NotFound => println!("no match with a cover"),
        Lookup::Failed(error) => println!("lookup failed: {error}"),
    }
}
