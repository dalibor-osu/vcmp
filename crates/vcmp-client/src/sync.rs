use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use vcmp_library::{FeedCursor, Format, Library, MirroredTrack, Server, ServerFolder};
use vcmp_protocol::{Cursor, FEED_PAGE_DEFAULT, Handshake, RemoteTrack};

use crate::{ClientError, Connection};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the address now answers as another server")]
    OtherServer,
    #[error("the library: {0}")]
    Library(#[from] vcmp_library::Error),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub changed: usize,
    pub from_start: bool,
    pub gone: usize,
    pub artwork: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncProgress {
    pub received: usize,
}

pub fn mirrored(track: &RemoteTrack) -> MirroredTrack {
    MirroredTrack {
        remote_id: track.id,
        available: track.available,
        folder: track.folder,
        path: track.path.clone(),
        format: track.format.as_deref().and_then(Format::parse),
        size: track.size,
        modified_ms: track.modified_ms,
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_artist: track.album_artist.clone(),
        track_number: track.track_number,
        disc_number: track.disc_number,
        year: track.year,
        genre: track.genre.clone(),
        duration: Duration::from_millis(track.duration_ms),
        added_ms: track.added_ms,
        artwork: track.artwork.clone(),
        mb_release_id: track.mb_release_id.clone(),
        mb_artist_id: track.mb_artist_id.clone(),
        mb_album_artist_id: track.mb_album_artist_id.clone(),
        mb_recording_id: track.mb_recording_id.clone(),
    }
}

pub fn sync(
    library: &mut Library,
    server: &Server,
    connection: &Connection,
    cancel: &AtomicBool,
    mut progress: impl FnMut(SyncProgress),
) -> Result<(Handshake, SyncReport), SyncError> {
    let handshake = connection.handshake()?;
    if handshake.server_id != server.server_id {
        return Err(SyncError::OtherServer);
    }
    let folders = connection.folders()?;
    library.set_server_folders(
        server.id,
        &folders
            .iter()
            .map(|folder| ServerFolder {
                id: folder.id,
                name: folder.name.clone(),
            })
            .collect::<Vec<_>>(),
    )?;

    let mut report = SyncReport::default();
    let mut cursor = Cursor {
        revision: server.cursor.revision,
        track: server.cursor.track,
    };
    let mut seen = HashSet::new();
    report.from_start = cursor == Cursor::default();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(SyncError::Cancelled);
        }
        let page = connection.feed(cursor, FEED_PAGE_DEFAULT)?;
        if page.head < cursor {
            log::info!(
                "{} is behind this app's copy (restored from a backup?); syncing from the start",
                server.name
            );
            library.restart_server_feed(server.id)?;
            cursor = Cursor::default();
            report.from_start = true;
            seen.clear();
            continue;
        }
        let tracks: Vec<MirroredTrack> = page.tracks.iter().map(mirrored).collect();
        seen.extend(tracks.iter().map(|track| track.remote_id));
        library.apply_server_page(
            server.id,
            &tracks,
            FeedCursor {
                revision: page.next.revision,
                track: page.next.track,
            },
        )?;
        report.changed += tracks.len();
        progress(SyncProgress {
            received: report.changed,
        });
        cursor = page.next;
        if !page.more {
            break;
        }
    }
    if report.from_start {
        report.gone = library.forget_unseen_server_tracks(server.id, &seen)?;
    }
    library.finish_server_sync(server.id)?;

    for key in library.missing_server_artwork(server.id)? {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match (connection.artwork(&key), connection.thumbnail(&key)) {
            (Ok(original), Ok(thumbnail)) => {
                if library
                    .store_server_artwork(&key, &original, &thumbnail)?
                    .is_some()
                {
                    report.artwork += 1;
                }
            }
            (Err(error), _) | (_, Err(error)) => {
                if error.is_unreachable() {
                    return Err(error.into());
                }
                log::warn!("the cover {key} could not be fetched: {error}");
            }
        }
    }
    Ok((handshake, report))
}
