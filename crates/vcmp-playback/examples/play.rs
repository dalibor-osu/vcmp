//! Plays files through the real output device and prints the engine's events, for
//! checking playback by ear (or silently, with `--volume 0`).
//!
//! ```text
//! cargo run -p vcmp-playback --example play -- --devices
//! cargo run -p vcmp-playback --example play -- [options] <file or directory>...
//!   --device NAME     output device (default: the system default)
//!   --volume V        0.0 to 1.0 (default 0.5)
//!   --shuffle MODE    true | less-repeats
//!   --start-at S      seek to S seconds into the first track
//!   --seconds S       quit after S seconds
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vcmp_playback::{
    Command, Event, Player, PlayerOptions, QueueEntry, ShuffleMode, TrackRef, output_devices,
};

const EXTENSIONS: [&str; 4] = ["mp3", "flac", "wav", "ogg"];

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for entry in entries {
            collect(&entry, files);
        }
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    {
        files.push(path.to_owned());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut options = PlayerOptions {
        volume: 0.5,
        ..PlayerOptions::default()
    };
    let mut shuffle = None;
    let mut start_at = None;
    let mut seconds = None;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--devices" => {
                println!("System default");
                for name in output_devices()? {
                    println!("{name}");
                }
                return Ok(());
            }
            "--device" => options.output_device = args.next(),
            "--volume" => options.volume = args.next().ok_or("--volume needs a value")?.parse()?,
            "--shuffle" => {
                shuffle = Some(match args.next().as_deref() {
                    Some("true") => ShuffleMode::True,
                    Some("less-repeats") => ShuffleMode::LessRepeats,
                    _ => return Err("--shuffle takes true or less-repeats".into()),
                })
            }
            "--start-at" => {
                start_at = Some(
                    args.next()
                        .ok_or("--start-at needs a value")?
                        .parse::<f64>()?,
                )
            }
            "--seconds" => {
                seconds = Some(
                    args.next()
                        .ok_or("--seconds needs a value")?
                        .parse::<f64>()?,
                )
            }
            path => collect(Path::new(path), &mut files),
        }
    }
    if files.is_empty() {
        return Err("no playable files given".into());
    }

    let (player, events) = Player::new(options)?;
    if let Some(mode) = shuffle {
        player.send(Command::SetShuffle(mode));
    }
    let entries = files
        .iter()
        .map(|path| QueueEntry::new(TrackRef::local(path.to_string_lossy().as_ref())))
        .collect();
    player.send(Command::PlayContext {
        entries,
        origin: None,
        start: None,
    });
    if let Some(start) = start_at {
        player.send(Command::Seek(Duration::from_secs_f64(start)));
    }

    let began = Instant::now();
    let deadline = seconds.map(|s| began + Duration::from_secs_f64(s));
    loop {
        let wait = deadline.map_or(Duration::from_secs(3600), |d| {
            d.saturating_duration_since(Instant::now())
        });
        let Ok(event) = events.recv_timeout(wait) else {
            break;
        };
        let at = began.elapsed().as_secs_f64();
        match &event {
            Event::Position(position) => {
                println!("{at:7.3}s  position {:.3}s", position.as_secs_f64())
            }
            Event::QueueChanged(_) => {}
            Event::TrackChanged(Some(now)) => println!(
                "{at:7.3}s  track {} ({:.3}s)",
                now.entry.track.id,
                now.duration.unwrap_or_default().as_secs_f64()
            ),
            Event::TrackChanged(None) => {
                println!("{at:7.3}s  end of queue");
                break;
            }
            other => println!("{at:7.3}s  {other:?}"),
        }
    }
    if let Some(snapshot) = player.snapshot() {
        println!("stopped at {:.3}s", snapshot.position.as_secs_f64());
    }
    Ok(())
}
