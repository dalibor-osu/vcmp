use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use vcmp_client::{Connection, DEFAULT_CACHE_LIMIT, ServerSource, sync};
use vcmp_library::Library;
use vcmp_playback::{
    Command, Event, Player, PlayerOptions, QueueEntry, SourceKind, Sources, TrackRef,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let music = PathBuf::from(
        args.next()
            .ok_or("usage: play_server <music folder> [--volume V]")?,
    );
    let mut volume = 0.0;
    while let Some(arg) = args.next() {
        if arg == "--volume" {
            volume = args.next().ok_or("--volume needs a value")?.parse()?;
        }
    }
    let work = tempfile::tempdir()?;
    let mut config = vcmp_server::Config::new(work.path().join("server"));
    config.listen = "127.0.0.1:0".parse()?;
    config.music = vec![music];
    config.rescan_interval = None;
    let (_, tokens) = vcmp_server::tokens(&config)?;
    let server = vcmp_server::Server::start(config)?;
    let connection = Connection::new(&format!("http://{}", server.address()), &tokens[0].token)?;
    while connection
        .scan_status()
        .map(|status| status.running || status.last.is_none())
        .unwrap_or(true)
    {
        std::thread::sleep(Duration::from_millis(100));
    }

    let database = work.path().join("app/library.sqlite3");
    let cache = work.path().join("app/cache");
    let mut library = Library::open(&database, &cache)?;
    let handshake = connection.handshake()?;
    let row = library.add_server(&handshake.server_id, &handshake.name, connection.address())?;
    let (_, report) = sync(
        &mut library,
        &row,
        &connection,
        &AtomicBool::new(false),
        |_| {},
    )?;
    println!("synced {} tracks", report.changed);

    let source = ServerSource::new(
        cache.join("streams"),
        Library::open(&database, &cache)?,
        DEFAULT_CACHE_LIMIT,
    )?;
    source.set_connection(row.id, Some(connection.clone()));
    let entries: Vec<QueueEntry> = library
        .tracks()?
        .iter()
        .map(|track| QueueEntry::new(TrackRef::server(track.key())))
        .collect();
    println!("playing {} tracks at volume {volume}", entries.len());
    let (player, events) = Player::new(PlayerOptions {
        sources: Sources::default().with(SourceKind::Server, Arc::new(source)),
        volume,
        ..PlayerOptions::default()
    })?;
    player.send(Command::PlayContext {
        entries,
        origin: None,
        start: None,
    });
    let started = Instant::now();
    let mut changes = 0;
    while let Ok(event) = events.recv_timeout(Duration::from_secs(30)) {
        let at = started.elapsed().as_secs_f64();
        match event {
            Event::Position(position) => {
                if position.subsec_millis() < 260 {
                    println!("{at:6.2} s  position {:.2} s", position.as_secs_f64());
                }
            }
            Event::TrackChanged(Some(now)) => {
                changes += 1;
                println!(
                    "{at:6.2} s  started {} ({:?})",
                    now.entry.track.id, now.duration
                );
            }
            Event::TrackChanged(None) => {
                println!("{at:6.2} s  the queue ended");
                break;
            }
            other => println!("{at:6.2} s  {other:?}"),
        }
    }
    println!("{changes} tracks started");
    Ok(())
}
