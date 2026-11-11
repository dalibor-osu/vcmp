use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, Tag, TagExt};
use tempfile::TempDir;
use vcmp_client::{Connection, sync};
use vcmp_library::{FeedCursor, Library, Server};
use vcmp_protocol::{FieldEdit, TagEdit, TagWriteGroup, TagWriteRequest};
use vcmp_server::Config;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vcmp-library/tests/fixtures"
);

fn add_song(dir: &Path, name: &str, fixture: &str, title: &str, artist: &str, album: &str) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
    let file = lofty::read_from_path(&path).unwrap();
    let mut tag = Tag::new(file.primary_tag_type());
    tag.set_title(title.into());
    tag.set_artist(artist.into());
    tag.set_album(album.into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(16, 16, image::Rgb([20, 120, 220]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

struct Setup {
    server: vcmp_server::Server,
    library: Library,
    user: Connection,
    admin: Connection,
    base: PathBuf,
    _dir: TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let album = base.join("server-music/Album");
    add_song(&album, "1.flac", "silence.flac", "One", "Artist", "Album");
    add_song(&album, "2.flac", "silence.flac", "Two", "Artist", "Album");
    fs::write(album.join("cover.png"), png()).unwrap();
    add_song(
        &base.join("server-music/Loose"),
        "3.flac",
        "silence.flac",
        "Three",
        "Other",
        "Elsewhere",
    );

    let mut config = Config::new(base.join("server-data"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.music = vec![base.join("server-music")];
    config.rescan_interval = None;
    let (_, tokens) = vcmp_server::tokens(&config).unwrap();
    let server = vcmp_server::Server::start(config).unwrap();
    let address = format!("http://{}", server.address());
    let user = Connection::new(&address, &tokens[0].token).unwrap();
    let admin = Connection::new(&address, &tokens[1].token).unwrap();
    let start = Instant::now();
    while user
        .scan_status()
        .map(|status| status.running || status.last.is_none())
        .unwrap_or(true)
    {
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(50));
    }
    let library =
        Library::open(&base.join("app/library.sqlite3"), &base.join("app/cache")).unwrap();
    Setup {
        server,
        library,
        user,
        admin,
        base,
        _dir: dir,
    }
}

impl Setup {
    fn connect(&self) -> Server {
        let handshake = self.user.handshake().unwrap();
        self.library
            .add_server(&handshake.server_id, &handshake.name, self.user.address())
            .unwrap()
    }

    fn sync(&mut self) -> vcmp_client::SyncReport {
        let server = self.library.servers().unwrap().remove(0);
        sync(
            &mut self.library,
            &server,
            &self.user,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
        .1
    }
}

fn titles(library: &Library) -> Vec<String> {
    let mut titles: Vec<String> = library
        .tracks()
        .unwrap()
        .into_iter()
        .map(|track| track.title)
        .collect();
    titles.sort();
    titles
}

#[test]
fn a_synced_server_joins_the_library_with_its_albums_and_covers() {
    let mut setup = setup();
    let server = setup.connect();
    assert_eq!(server.server_id, setup.server.server_id());
    let report = setup.sync();
    assert_eq!(report.changed, 3);
    assert!(report.from_start);
    assert_eq!(report.artwork, 1);

    assert_eq!(titles(&setup.library), ["One", "Three", "Two"]);
    let albums = setup.library.albums().unwrap();
    assert_eq!(albums.len(), 2);
    let album = albums.iter().find(|album| album.title == "Album").unwrap();
    assert_eq!(album.server, Some(server.id));
    assert_eq!(album.track_count, 2);
    let artwork = album.artwork.as_ref().expect("the server's cover, cached");
    assert_eq!(fs::read(&artwork.original).unwrap(), png());
    assert!(artwork.thumbnail.is_file());

    let one = setup
        .library
        .tracks()
        .unwrap()
        .into_iter()
        .find(|track| track.title == "One")
        .unwrap();
    assert_eq!(one.file, None);
    let remote = one.remote.clone().unwrap();
    assert_eq!(remote.server, server.id);
    assert_eq!(remote.path, "Album/1.flac");
    assert_eq!(
        setup.library.remote_track(server.id, remote.id).unwrap(),
        Some(one.id)
    );
    assert_eq!(setup.library.server_folders(server.id).unwrap().len(), 1);
    let found = setup.library.search("Three", 10).unwrap();
    assert_eq!(found.tracks.len(), 1);

    assert_eq!(setup.sync().changed, 0);
}

#[test]
fn a_local_scan_leaves_the_mirror_alone() {
    let mut setup = setup();
    setup.connect();
    setup.sync();
    let before: Vec<_> = setup.library.tracks().unwrap();
    let albums_before = setup.library.albums().unwrap();

    let local = setup.base.join("local-music");
    add_song(&local, "1.flac", "silence.flac", "One", "Artist", "Album");
    setup.library.add_source(&local, true).unwrap();
    let report = setup.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(report.relinked, 0);
    assert_eq!(report.became_unavailable, 0);

    let after = setup.library.tracks().unwrap();
    for track in &before {
        assert!(after.contains(track), "{track:?} changed");
    }
    let albums_after = setup.library.albums().unwrap();
    for album in &albums_before {
        assert!(albums_after.contains(album), "{album:?} changed");
    }
    let named_album: Vec<_> = albums_after
        .iter()
        .filter(|album| album.title == "Album")
        .collect();
    assert_eq!(
        named_album.len(),
        2,
        "a local and a server album stay apart"
    );
    assert_eq!(
        setup
            .library
            .artists()
            .unwrap()
            .iter()
            .filter(|a| a.name == "Artist")
            .count(),
        1,
        "artists are shared"
    );
    assert!(setup.library.duplicate_groups().unwrap().is_empty());
    assert!(
        setup
            .library
            .missing_tags()
            .unwrap()
            .iter()
            .all(|gaps| gaps.tracks.iter().all(|gap| gap.track.file.is_some()))
    );
}

#[test]
fn changes_on_the_server_arrive_and_removal_keeps_the_rows() {
    let mut setup = setup();
    let server = setup.connect();
    setup.sync();
    let one = setup
        .library
        .tracks()
        .unwrap()
        .into_iter()
        .find(|track| track.title == "One")
        .unwrap();
    let remote = one.remote.clone().unwrap();
    let result = setup
        .admin
        .write_tags(&TagWriteRequest {
            groups: vec![TagWriteGroup {
                tracks: vec![remote.id],
                edit: TagEdit {
                    title: FieldEdit::Set("Uno".into()),
                    ..TagEdit::default()
                },
            }],
        })
        .unwrap();
    assert_eq!(result.written, [remote.id]);
    let report = setup.sync();
    assert_eq!(report.changed, 1);
    let uno = setup.library.track(one.id).unwrap().unwrap();
    assert_eq!(uno.title, "Uno");

    let playlist = setup.library.create_playlist("Mixed").unwrap();
    setup.library.add_to_playlist(playlist, &[one.id]).unwrap();
    setup.library.like(&[one.id]).unwrap();

    setup.library.remove_server(server.id).unwrap();
    assert!(setup.library.servers().unwrap().is_empty());
    assert!(setup.library.tracks().unwrap().is_empty());
    assert!(!setup.library.track(one.id).unwrap().unwrap().available);

    setup.connect();
    let report = setup.sync();
    assert!(report.from_start);
    assert_eq!(
        setup.library.track(one.id).unwrap().unwrap().title,
        "Uno",
        "the same row comes back"
    );
    let entries = setup.library.playlist_entries(playlist).unwrap();
    assert!(entries[0].track.available);
    assert!(setup.library.liked_tracks().unwrap().contains(&one.id));
}

#[test]
fn an_app_ahead_of_a_restored_server_syncs_again_from_the_start() {
    let mut setup = setup();
    let server = setup.connect();
    setup.sync();
    setup
        .library
        .apply_server_page(
            server.id,
            &[],
            FeedCursor {
                revision: 1_000_000,
                track: 0,
            },
        )
        .unwrap();
    let report = setup.sync();
    assert!(report.from_start);
    assert_eq!(report.changed, 3);
    assert_eq!(report.gone, 0);
    assert_eq!(titles(&setup.library), ["One", "Three", "Two"]);
}

fn decoded(stream: &mut Box<dyn vcmp_playback::TrackStream>) -> Vec<f32> {
    let mut samples = Vec::new();
    while let Some(chunk) = stream.next_chunk().unwrap() {
        samples.extend_from_slice(chunk.samples);
    }
    samples
}

#[test]
fn a_server_track_plays_through_the_server_source_as_the_file_does() {
    use vcmp_playback::{LocalSource, PlaybackError, TrackSource};

    let mut setup = setup();
    let server = setup.connect();
    setup.sync();
    let one = setup
        .library
        .tracks()
        .unwrap()
        .into_iter()
        .find(|track| track.title == "One")
        .unwrap();
    let lookup = Library::open(
        &setup.base.join("app/library.sqlite3"),
        &setup.base.join("app/cache"),
    )
    .unwrap();
    let source = vcmp_client::ServerSource::new(
        setup.base.join("app/cache/streams"),
        lookup,
        vcmp_client::DEFAULT_CACHE_LIMIT,
    )
    .unwrap();
    let key = one.key();
    source.set_connection(server.id, Some(setup.user.clone()));
    assert!(matches!(source.open(&key), Err(PlaybackError::NotReady)));

    let start = Instant::now();
    let mut stream = loop {
        match source.open(&key) {
            Ok(stream) => break stream,
            Err(PlaybackError::NotReady) => {
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "never downloaded"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("{error}"),
        }
    };
    let original = setup.base.join("server-music/Album/1.flac");
    let mut local = LocalSource.open(original.to_str().unwrap()).unwrap();
    assert_eq!(decoded(&mut stream), decoded(&mut local));
    assert!(source.is_cached(&key));
    assert!(source.size() > 0);
    drop(stream);

    source.clear();
    assert!(!source.is_cached(&key));
    source.set_connection(server.id, None);
    assert!(matches!(source.open(&key), Err(PlaybackError::NotReady)));
    let start = Instant::now();
    loop {
        match source.open(&key) {
            Err(PlaybackError::NotReady) => {
                assert!(start.elapsed() < Duration::from_secs(30));
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
            Ok(_) => panic!("played without a connection"),
        }
    }
}
