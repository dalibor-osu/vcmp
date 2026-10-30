//! Animated pictures (GIF, WebP, APNG) as a playlist's picture, an artist's photo and
//! an album's cover: kept as they are beside the still everything else shows.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, Rgba, RgbaImage};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use tempfile::TempDir;
use vcmp_library::animation::{self, decode_frames, first_frame_png};
use vcmp_library::{FieldEdit, Library, TagEdit};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Setup {
    // The database closes before the directory is deleted, which Windows requires.
    library: Library,
    base: PathBuf,
    _root: TempDir,
}

impl Setup {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(root.path()).unwrap();
        fs::create_dir(base.join("music")).unwrap();
        let library =
            Library::open(&base.join("data/library.sqlite3"), &base.join("cache")).unwrap();
        library.add_source(&base.join("music"), true).unwrap();
        Self {
            _root: root,
            base,
            library,
        }
    }

    /// Copies a fixture into `music/<album>/` with an artist and album tag.
    fn add_track(&self, album: &str, name: &str, fixture: &str) -> PathBuf {
        let dir = self.base.join("music").join(album);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
        let file = lofty::read_from_path(&path).unwrap();
        let mut tag = Tag::new(file.primary_tag_type());
        tag.set_title(name.into());
        tag.set_artist("Artist".into());
        tag.set_album(album.into());
        tag.insert_text(ItemKey::AlbumArtist, "Artist".into());
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        path
    }

    fn scan(&mut self) {
        let report = self.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    fn file(&self, name: &str, data: &[u8]) -> PathBuf {
        let path = self.base.join(name);
        fs::write(&path, data).unwrap();
        path
    }
}

/// A looping GIF of one frame per colour, 48 px square.
fn gif(colours: &[[u8; 3]]) -> Vec<u8> {
    let mut data = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut data);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        for &[r, g, b] in colours {
            let image = RgbaImage::from_pixel(48, 48, Rgba([r, g, b, 255]));
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(80, 1),
                ))
                .unwrap();
        }
    }
    data
}

fn jpeg(colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    bytes.into_inner()
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn a_playlist_keeps_its_gif_beside_the_still() {
    let setup = Setup::new();
    let playlist = setup.library.create_playlist("Moving").unwrap();
    let data = gif(&[RED, GREEN, BLUE]);
    setup
        .library
        .set_playlist_image(playlist, &setup.file("dance.gif", &data))
        .unwrap();

    let still = setup
        .library
        .playlist(playlist)
        .unwrap()
        .unwrap()
        .picture
        .expect("a picture");
    assert_eq!(still.extension().unwrap(), "jpg");
    let moving = animation::beside(&still).expect("the animation is kept");
    assert_eq!(moving.extension().unwrap(), "gif");
    assert_eq!(fs::read(&moving).unwrap(), data, "kept as it was");
    assert_eq!(decode_frames(&data, 100).unwrap().len(), 3);

    // A still picture in its place takes the animation with the old one.
    setup
        .library
        .set_playlist_image(playlist, &setup.file("still.jpg", &jpeg(BLUE)))
        .unwrap();
    assert!(!moving.exists() && !still.exists());
    let new = setup.library.playlist(playlist).unwrap().unwrap().picture;
    assert_eq!(animation::beside(&new.unwrap()), None);

    // Going back to covers, or deleting the playlist, deletes both files.
    setup
        .library
        .set_playlist_image(playlist, &setup.file("dance.gif", &data))
        .unwrap();
    let still = setup.library.playlist(playlist).unwrap().unwrap().picture;
    let still = still.unwrap();
    let moving = animation::beside(&still).unwrap();
    setup.library.clear_playlist_image(playlist).unwrap();
    assert!(!moving.exists() && !still.exists());

    setup
        .library
        .set_playlist_image(playlist, &setup.file("dance.gif", &data))
        .unwrap();
    let still = setup.library.playlist(playlist).unwrap().unwrap().picture;
    let moving = animation::beside(still.as_ref().unwrap()).unwrap();
    setup.library.delete_playlist(playlist).unwrap();
    assert!(!moving.exists() && !still.unwrap().exists());
}

#[test]
fn a_one_frame_gif_stays_a_still() {
    let setup = Setup::new();
    let playlist = setup.library.create_playlist("Still").unwrap();
    setup
        .library
        .set_playlist_image(playlist, &setup.file("still.gif", &gif(&[RED])))
        .unwrap();
    let still = setup.library.playlist(playlist).unwrap().unwrap().picture;
    assert_eq!(animation::beside(&still.unwrap()), None);
}

#[test]
fn an_artist_keeps_their_gif_beside_the_photo() {
    let mut setup = Setup::new();
    setup.add_track("Album", "one.flac", "silence.flac");
    setup.scan();
    let artist = setup.library.artists().unwrap()[0].id;
    let data = gif(&[GREEN, BLUE]);

    let still = setup.library.set_artist_photo(artist, &data, None).unwrap();
    assert_eq!(
        setup.library.artist_photo(artist).unwrap(),
        Some(still.clone())
    );
    let moving = animation::beside(&still).expect("the animation is kept");
    assert_eq!(fs::read(&moving).unwrap(), data);

    setup.library.clear_artist_photo(artist).unwrap();
    assert!(!moving.exists() && !still.exists());
}

#[test]
fn an_album_plays_the_animation_kept_for_its_cover() {
    let mut setup = Setup::new();
    let tracks = [
        setup.add_track("Album", "one.flac", "silence.flac"),
        setup.add_track("Album", "two.mp3", "silence.mp3"),
    ];
    setup.scan();
    let ids: Vec<_> = tracks
        .iter()
        .map(|path| setup.library.track_id_for_path(path).unwrap().unwrap())
        .collect();

    // The files get the first frame; VCMP keeps the animation for that still.
    let data = gif(&[RED, GREEN, BLUE]);
    let still = first_frame_png(&data).unwrap();
    assert!(setup.library.keep_cover_animation(&still, &data).unwrap());
    let edit = TagEdit {
        cover: FieldEdit::Set(still.clone()),
        ..TagEdit::default()
    };
    let edits: Vec<_> = ids.iter().map(|&id| (id, &edit)).collect();
    let report = setup
        .library
        .write_tags(&edits, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);

    let album = &setup.library.albums().unwrap()[0];
    let artwork = album.artwork.as_ref().expect("the still is the cover");
    let moving = setup
        .library
        .cover_animation(artwork)
        .expect("the album has its animation");
    assert!(moving.starts_with(setup.base.join("data/images/covers")));
    assert_eq!(fs::read(&moving).unwrap(), data);

    // Another cover ends it.
    let edit = TagEdit {
        cover: FieldEdit::Set(jpeg(BLUE)),
        ..TagEdit::default()
    };
    let edits: Vec<_> = ids.iter().map(|&id| (id, &edit)).collect();
    setup
        .library
        .write_tags(&edits, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    let album = &setup.library.albums().unwrap()[0];
    assert_eq!(
        setup
            .library
            .cover_animation(album.artwork.as_ref().unwrap()),
        None
    );
}

#[test]
fn a_folder_gif_is_the_cover_and_may_move() {
    let mut setup = Setup::new();
    let track = setup.add_track("Album", "one.flac", "silence.flac");
    let data = gif(&[RED, BLUE]);
    fs::write(track.with_file_name("cover.gif"), &data).unwrap();
    setup.scan();

    let album = &setup.library.albums().unwrap()[0];
    let artwork = album
        .artwork
        .as_ref()
        .expect("the folder's GIF is the cover");
    assert!(artwork.thumbnail.is_file());
    let moving = setup.library.cover_animation(artwork).unwrap();
    assert_eq!(moving, artwork.original);
    assert_eq!(
        decode_frames(&fs::read(&moving).unwrap(), 64)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_still_cover_has_no_animation() {
    let mut setup = Setup::new();
    let track = setup.add_track("Album", "one.flac", "silence.flac");
    fs::write(track.with_file_name("cover.jpg"), jpeg(GREEN)).unwrap();
    setup.scan();
    let album = &setup.library.albums().unwrap()[0];
    assert_eq!(
        setup
            .library
            .cover_animation(album.artwork.as_ref().unwrap()),
        None
    );
    // A still is never kept as an animation.
    assert!(
        !setup
            .library
            .keep_cover_animation(&jpeg(GREEN), &jpeg(RED))
            .unwrap()
    );
}
