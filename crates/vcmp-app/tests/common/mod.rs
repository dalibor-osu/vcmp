//! Helpers shared by the integration tests: tagged audio files made from the library
//! crate's silent fixtures.

// Each test file compiles this module on its own and uses only part of it.
#![allow(dead_code)]

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vcmp-library/tests/fixtures/silence.flac"
);

#[derive(Clone, Copy)]
pub struct Song<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub album_artist: Option<&'a str>,
    pub track: u32,
    pub disc: u32,
}

impl<'a> Song<'a> {
    pub fn new(title: &'a str, artist: &'a str, album: &'a str) -> Self {
        Self {
            title,
            artist,
            album,
            album_artist: None,
            track: 1,
            disc: 1,
        }
    }
}

/// Copies the silent FLAC fixture to `dir/name` (creating `dir`) and tags it.
pub fn add(dir: &Path, name: &str, song: Song) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::copy(FIXTURE, &path).unwrap();
    let file = lofty::read_from_path(&path).unwrap();
    let mut tag = Tag::new(file.primary_tag_type());
    tag.set_title(song.title.into());
    // An empty artist or album is left out of the tag.
    if !song.artist.is_empty() {
        tag.set_artist(song.artist.into());
    }
    if !song.album.is_empty() {
        tag.set_album(song.album.into());
    }
    tag.set_track(song.track);
    tag.set_disk(song.disc);
    if let Some(album_artist) = song.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, album_artist.into());
    }
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
}
