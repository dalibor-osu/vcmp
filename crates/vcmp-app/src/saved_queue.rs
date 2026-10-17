//! The queue and position kept across restarts: the engine's `PlaybackSnapshot` as
//! JSON in the library's settings. The format is VCMP's own, with a version, so the
//! engine's types can change without breaking saved queues.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vcmp_playback::{
    PlaybackSnapshot, QueueEntry, QueueSlot, QueueSnapshot, RepeatMode, ShuffleMode, SourceKind,
    TrackRef,
};

/// Setting key holding the saved queue.
pub const SETTING: &str = "playback.queue";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    position_ms: u64,
    context: Vec<Entry>,
    origin: Option<String>,
    order: Vec<usize>,
    cursor: usize,
    current: Option<Slot>,
    history: Vec<Slot>,
    up_next: Vec<Entry>,
    shuffle: Shuffle,
    repeat: Repeat,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    source: String,
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    album: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Slot {
    Context(usize),
    UpNext(Entry),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Shuffle {
    Off,
    True,
    LessRepeats,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Repeat {
    Off,
    All,
    One,
}

pub fn encode(snapshot: &PlaybackSnapshot) -> String {
    let queue = &snapshot.queue;
    let saved = Saved {
        version: VERSION,
        position_ms: snapshot.position.as_millis() as u64,
        context: queue.context.iter().map(save_entry).collect(),
        origin: queue.origin.clone(),
        order: queue.order.clone(),
        cursor: queue.cursor,
        current: queue.current.as_ref().map(save_slot),
        history: queue.history.iter().map(save_slot).collect(),
        up_next: queue.up_next.iter().map(save_entry).collect(),
        shuffle: match queue.shuffle {
            ShuffleMode::Off => Shuffle::Off,
            ShuffleMode::True => Shuffle::True,
            ShuffleMode::LessRepeats => Shuffle::LessRepeats,
        },
        repeat: match queue.repeat {
            RepeatMode::Off => Repeat::Off,
            RepeatMode::All => Repeat::All,
            RepeatMode::One => Repeat::One,
        },
    };
    serde_json::to_string(&saved).expect("a saved queue is plain data")
}

/// Reads a saved queue back. Fails on another version or an unknown track source.
pub fn decode(text: &str) -> Result<PlaybackSnapshot, String> {
    let saved: Saved = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if saved.version != VERSION {
        return Err(format!("unknown saved queue version {}", saved.version));
    }
    let load_slot = |slot: Slot| -> Result<QueueSlot, String> {
        Ok(match slot {
            Slot::Context(index) => QueueSlot::Context(index),
            Slot::UpNext(entry) => QueueSlot::UpNext(load_entry(entry)?),
        })
    };
    Ok(PlaybackSnapshot {
        queue: QueueSnapshot {
            context: saved
                .context
                .into_iter()
                .map(load_entry)
                .collect::<Result<_, _>>()?,
            origin: saved.origin,
            order: saved.order,
            cursor: saved.cursor,
            current: saved.current.map(load_slot).transpose()?,
            history: saved
                .history
                .into_iter()
                .map(load_slot)
                .collect::<Result<_, _>>()?,
            up_next: saved
                .up_next
                .into_iter()
                .map(load_entry)
                .collect::<Result<_, _>>()?,
            shuffle: match saved.shuffle {
                Shuffle::Off => ShuffleMode::Off,
                Shuffle::True => ShuffleMode::True,
                Shuffle::LessRepeats => ShuffleMode::LessRepeats,
            },
            repeat: match saved.repeat {
                Repeat::Off => RepeatMode::Off,
                Repeat::All => RepeatMode::All,
                Repeat::One => RepeatMode::One,
            },
        },
        position: Duration::from_millis(saved.position_ms),
    })
}

fn save_entry(entry: &QueueEntry) -> Entry {
    Entry {
        source: entry.track.source.as_str().to_string(),
        id: entry.track.id.to_string(),
        artist: entry.artist.as_deref().map(str::to_string),
        album: entry.album.as_deref().map(str::to_string),
    }
}

fn save_slot(slot: &QueueSlot) -> Slot {
    match slot {
        QueueSlot::Context(index) => Slot::Context(*index),
        QueueSlot::UpNext(entry) => Slot::UpNext(save_entry(entry)),
    }
}

fn load_entry(entry: Entry) -> Result<QueueEntry, String> {
    let source: SourceKind = entry.source.parse().map_err(|error| format!("{error}"))?;
    Ok(QueueEntry {
        track: TrackRef {
            source,
            id: Arc::from(entry.id),
        },
        artist: entry.artist.map(Arc::from),
        album: entry.album.map(Arc::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> QueueEntry {
        QueueEntry::new(TrackRef::local(path))
            .with_artist("3")
            .with_album("7")
    }

    #[test]
    fn a_snapshot_comes_back_unchanged() {
        let snapshot = PlaybackSnapshot {
            queue: QueueSnapshot {
                context: vec![entry("/m/1.flac"), entry("/m/2.flac"), entry("/m/3.flac")],
                origin: Some("album:7".into()),
                order: vec![2, 0, 1],
                cursor: 1,
                current: Some(QueueSlot::UpNext(QueueEntry::new(TrackRef::local(
                    "/m/x.mp3",
                )))),
                history: vec![QueueSlot::Context(0)],
                up_next: vec![entry("/m/y.ogg")],
                shuffle: ShuffleMode::LessRepeats,
                repeat: RepeatMode::One,
            },
            position: Duration::from_millis(83_250),
        };
        assert_eq!(decode(&encode(&snapshot)), Ok(snapshot));
    }

    #[test]
    fn unreadable_saves_are_refused() {
        assert!(decode("not json").is_err());
        let other_version = encode(&PlaybackSnapshot::default())
            .replace(&format!("\"version\":{VERSION}"), "\"version\":99");
        assert!(decode(&other_version).is_err());
        let other_source = encode(&PlaybackSnapshot {
            queue: QueueSnapshot {
                context: vec![entry("/m/1.flac")],
                ..QueueSnapshot::default()
            },
            ..PlaybackSnapshot::default()
        })
        .replace("\"local\"", "\"spotify\"");
        assert!(decode(&other_source).is_err());
    }
}
