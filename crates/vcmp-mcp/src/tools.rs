//! The tools agents call. Reads go to the server's own library connection; changes go
//! through [`Changes`], to the app. Every answer is JSON text.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vcmp_library::{
    Album, AlbumId, Artist, ChangeKind, ChangeSet, ChangeSetId, ChangeSetSummary, ChangeStatus,
    FieldEdit, FileResult, LABEL_PALETTE, Label, LabelEdit, Library, NewLabel, Playlist,
    PlaylistId, TagEdit, Track, TrackId, valid_label_colour,
};

use crate::{Changes, PlaylistEdit};

/// The most files one change of tags may cover, so the owner can still review it.
pub const MAX_FILES: usize = 500;
/// The most items one answer lists.
const MAX_PAGE: u32 = 500;
const DEFAULT_PAGE: u32 = 100;

/// What every agent is told about VCMP when it connects.
const INSTRUCTIONS: &str = "\
VCMP is the owner's music player. These tools read their music library and ask for \
changes to it.

Reading: search, list_albums, list_artists, get_album, get_tracks, list_tag_keys, \
find_tracks_by_tag, list_labels, find_tracks_by_labels, list_playlists and \
get_playlist. get_tracks lists every tag a \
file holds, by key: lofty's common name (TrackTitle, Composer, MusicBrainzReleaseId, \
ReplayGainTrackGain) or the format's own key behind a prefix (id3:TXXX:MY FIELD, \
vorbis:MYFIELD). The same keys are used to change tags.

Changing tags never happens at once. propose_tag_changes records a change set that \
waits until the owner reviews it in VCMP and accepts or rejects it, as a whole. Check \
get_change_set for its status: pending, accepted, partly_written (some files had \
changed since you looked and were left alone, or could not be written), rejected, \
discarded (VCMP closed before the owner decided) or undone. An accepted change can be \
undone with propose_undo, which is reviewed the same way. Changes are kept 30 days.

The nine fields VCMP shows have their own names in propose_tag_changes, and change \
what the owner sees:
- title: the track's name everywhere. A file without one shows its file name.
- artist: the track's artist. Credited artists get their own page.
- album: groups tracks into albums, together with album_artist. Tracks without an \
album go to their artist's Unknown album.
- album_artist: whose album it is: the album grid and artist pages show it. Without \
one, the track artist is used, so a compilation needs the same album_artist on every \
track to stay one album.
- track_number and disc_number: the order within an album.
- year: the album's year is its earliest track's.
- genre: shown and searchable.
- cover: the front cover embedded in the file, which VCMP shows for its album: a \
JPEG or PNG image from a file on this computer ({\"file\": \"/absolute/path.jpg\"}) \
or from the web ({\"url\": \"https://...\"}), which VCMP downloads, at most 10 MB. \
Give every track of an album the same cover. remove: [\"cover\"] takes it away. The owner \
sees the image before accepting.
A MusicBrainzReleaseId tag groups an album by itself, ahead of album and \
album_artist: tracks of one album must share it. Keep changes small and focused, and \
say in your own words what you asked for; the owner sees every value before and \
after.

Labels are the owner's own names for tracks, such as chill or workout, each with a \
colour. They are not tags: they live only in VCMP and never in the files, and \
find_tracks_by_tag doesn't see them. Every track you're given lists its labels. When \
the owner asks for music by mood or purpose (\"play some chill music\"), look at \
list_labels first: a label of that name is their own choice of tracks. \
propose_label_changes puts labels on tracks and takes them off, and can create new \
labels; like tag changes it waits for the owner's review, and propose_undo undoes it. \
Labelling an album labels the tracks it has now.

Playlists: create_playlist, rename_playlist, add_to_playlist, remove_from_playlist \
and move_in_playlist change playlists at once. Deleting one is never immediate: \
propose_playlist_deletion waits for the owner, and a deleted playlist can't be \
restored.";

#[derive(Clone)]
pub struct VcmpServer {
    library: Arc<Mutex<Library>>,
    changes: Arc<dyn Changes>,
    tool_router: ToolRouter<Self>,
}

impl VcmpServer {
    pub fn new(library: Arc<Mutex<Library>>, changes: Arc<dyn Changes>) -> Self {
        Self {
            library,
            changes,
            tool_router: Self::tool_router(),
        }
    }

    /// Runs `read` on the library, off the server's thread.
    async fn read<T: Serialize + Send + 'static>(
        &self,
        read: impl FnOnce(&Library) -> Result<T, String> + Send + 'static,
    ) -> Result<String, String> {
        let library = self.library.clone();
        let result = tokio::task::spawn_blocking(move || {
            let library = library
                .lock()
                .map_err(|_| "the library connection broke".to_owned())?;
            read(&library)
        })
        .await
        .map_err(|error| error.to_string())??;
        json(&result)
    }

    /// Hands a change to the app, off the server's thread.
    async fn change<T: Serialize + Send + 'static>(
        &self,
        change: impl FnOnce(&dyn Changes) -> Result<T, String> + Send + 'static,
    ) -> Result<String, String> {
        let changes = self.changes.clone();
        let result = tokio::task::spawn_blocking(move || change(changes.as_ref()))
            .await
            .map_err(|error| error.to_string())??;
        json(&result)
    }
}

fn json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

fn failed(error: vcmp_library::Error) -> String {
    error.to_string()
}

/// How the history names who asked: the client's own title or name.
fn agent(context: &RequestContext<RoleServer>) -> String {
    context
        .client_info()
        .map(|info| info.title.unwrap_or(info.name))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "An AI agent".to_owned())
}

fn page(offset: Option<u32>, limit: Option<u32>) -> (usize, usize) {
    (
        offset.unwrap_or(0) as usize,
        limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE) as usize,
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Words to find in titles, artists and albums, ignoring case and diacritics.
    text: String,
    /// The most results of each kind (default 20).
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PageArgs {
    /// How many to skip (default 0).
    offset: Option<u32>,
    /// How many to list (default 100, at most 500).
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AlbumArgs {
    album_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct TracksArgs {
    track_ids: Vec<i64>,
    /// List every tag each file holds (default true).
    include_tags: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ByTagArgs {
    /// A key as get_tracks lists it, such as MusicBrainzReleaseId or Genre.
    key: String,
    /// Only tracks with this value among the key's values.
    value: Option<String>,
    /// Instead, the tracks that don't hold the key at all.
    missing: Option<bool>,
    offset: Option<u32>,
    /// Default 100, at most 500.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ByLabelsArgs {
    /// Label names, ignoring case. A track must hold every one of them.
    labels: Vec<String>,
    offset: Option<u32>,
    /// Default 100, at most 500.
    limit: Option<u32>,
}

/// Labels to put on and take off a group of tracks.
#[derive(Deserialize, JsonSchema)]
pub struct LabelChange {
    track_ids: Vec<i64>,
    /// Label names to put on these tracks: existing labels, or ones in new_labels.
    #[serde(default)]
    add: Vec<String>,
    /// Label names to take off these tracks.
    #[serde(default)]
    remove: Vec<String>,
}

/// A label to create when the owner accepts the change.
#[derive(Deserialize, JsonSchema)]
pub struct NewLabelArgs {
    /// 1 to 40 characters, not the name of an existing label (ignoring case).
    name: String,
    /// One of pink, mauve, red, peach, yellow, green, teal, sky, blue, lavender (drawn
    /// to suit VCMP's light and dark themes), or any colour as #rrggbb.
    colour: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProposeLabelsArgs {
    /// Groups of tracks with the labels they get and lose. A track in several groups
    /// gets all of their changes; at most 500 tracks in all.
    changes: Vec<LabelChange>,
    /// Labels to create, used by name in changes.
    #[serde(default)]
    new_labels: Vec<NewLabelArgs>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlaylistArgs {
    playlist_id: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    TrackNumber,
    DiscNumber,
    Year,
    Genre,
    Cover,
}

/// Where a new front cover comes from.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CoverSource {
    /// The absolute path of a JPEG or PNG file on this computer.
    File(String),
    /// The http or https address of a JPEG or PNG image, which VCMP downloads.
    Url(String),
}

/// The changes to one track's file. Fields left out stay as they are.
#[derive(Deserialize, JsonSchema)]
pub struct TrackChange {
    track_id: i64,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    /// From 1 to 9999.
    year: Option<i32>,
    genre: Option<String>,
    /// A new front cover: {"file": "/absolute/path.jpg"} or {"url": "https://..."},
    /// a JPEG or PNG image of at most 10 MB.
    cover: Option<CoverSource>,
    /// Fields to remove from the file.
    #[serde(default)]
    remove: Vec<Field>,
    /// Other tags by key (as get_tracks lists them), each replaced by the values given,
    /// in order. An empty list removes the tag. The nine fields' own keys are refused:
    /// change those through the fields above.
    #[serde(default)]
    tags: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProposeArgs {
    /// One entry per track, at most 500.
    changes: Vec<TrackChange>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChangeArgs {
    change_set_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListChangesArgs {
    /// The newest this many (default 20).
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct NameArgs {
    name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RenameArgs {
    playlist_id: i64,
    name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddArgs {
    playlist_id: i64,
    /// Appended in this order.
    track_ids: Vec<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RemoveArgs {
    playlist_id: i64,
    /// Entry positions, from 0, as get_playlist lists them.
    positions: Vec<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct MoveArgs {
    playlist_id: i64,
    /// The entry's position now, from 0.
    from: u32,
    /// The position it moves to.
    to: u32,
}

#[tool_router]
impl VcmpServer {
    #[tool(
        description = "Find artists, albums and tracks whose title, artist or album holds every word, ignoring case and diacritics."
    )]
    async fn search(&self, Parameters(args): Parameters<SearchArgs>) -> Result<String, String> {
        let limit = args.limit.unwrap_or(20).clamp(1, MAX_PAGE) as usize;
        self.read(move |library| {
            let found = library.search(&args.text, limit).map_err(failed)?;
            Ok(serde_json::json!({
                "artists": found.artists.iter().map(artist_out).collect::<Vec<_>>(),
                "albums": found.albums.iter().map(album_out).collect::<Vec<_>>(),
                "tracks": tracks_out(library, &found.tracks)?,
            }))
        })
        .await
    }

    #[tool(
        description = "List the albums, by album artist and title, with their ids, years and track counts."
    )]
    async fn list_albums(&self, Parameters(args): Parameters<PageArgs>) -> Result<String, String> {
        let (offset, limit) = page(args.offset, args.limit);
        self.read(move |library| {
            let albums = library.albums().map_err(failed)?;
            Ok(serde_json::json!({
                "total": albums.len(),
                "albums": albums.iter().skip(offset).take(limit).map(album_out).collect::<Vec<_>>(),
            }))
        })
        .await
    }

    #[tool(description = "List every artist with their number of albums and tracks.")]
    async fn list_artists(&self, Parameters(args): Parameters<PageArgs>) -> Result<String, String> {
        let (offset, limit) = page(args.offset, args.limit);
        self.read(move |library| {
            let artists = library.artists().map_err(failed)?;
            Ok(serde_json::json!({
                "total": artists.len(),
                "artists": artists.iter().skip(offset).take(limit).map(artist_out).collect::<Vec<_>>(),
            }))
        })
        .await
    }

    #[tool(description = "One album and its tracks in order, with the fields VCMP shows.")]
    async fn get_album(&self, Parameters(args): Parameters<AlbumArgs>) -> Result<String, String> {
        self.read(move |library| {
            let id = AlbumId(args.album_id);
            let album = library
                .album(id)
                .map_err(failed)?
                .ok_or_else(|| format!("there is no album {}", args.album_id))?;
            let tracks = library.album_tracks(id).map_err(failed)?;
            Ok(serde_json::json!({
                "album": album_out(&album),
                "tracks": tracks_out(library, &tracks)?,
            }))
        })
        .await
    }

    #[tool(
        description = "Tracks by id: the fields VCMP shows, the file, whether it is available, and every tag the file holds by key, as the last scan read it."
    )]
    async fn get_tracks(&self, Parameters(args): Parameters<TracksArgs>) -> Result<String, String> {
        if args.track_ids.len() > MAX_PAGE as usize {
            return Err(format!("ask for at most {MAX_PAGE} tracks at a time"));
        }
        let with_tags = args.include_tags.unwrap_or(true);
        self.read(move |library| {
            let mut tracks = Vec::with_capacity(args.track_ids.len());
            for id in args.track_ids {
                let track = library
                    .track(TrackId(id))
                    .map_err(failed)?
                    .ok_or_else(|| format!("there is no track {id}"))?;
                let tags = if with_tags {
                    let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
                    for entry in library.track_tags(track.id).map_err(failed)? {
                        tags.entry(entry.key).or_default().push(entry.value);
                    }
                    Some(tags)
                } else {
                    None
                };
                tracks.push(track_out(library, &track, tags)?);
            }
            Ok(tracks)
        })
        .await
    }

    #[tool(
        description = "Every tag key the library's files hold, with how many tracks hold it, the most common first."
    )]
    async fn list_tag_keys(&self) -> Result<String, String> {
        self.read(|library| {
            let keys = library.tag_keys().map_err(failed)?;
            Ok(keys
                .into_iter()
                .map(|(key, tracks)| serde_json::json!({ "key": key, "tracks": tracks }))
                .collect::<Vec<_>>())
        })
        .await
    }

    #[tool(
        description = "Available tracks holding a tag key (with a value, when given), or with missing true those without it, such as every track without a MusicBrainzReleaseId."
    )]
    async fn find_tracks_by_tag(
        &self,
        Parameters(args): Parameters<ByTagArgs>,
    ) -> Result<String, String> {
        let (offset, limit) = page(args.offset, args.limit);
        self.read(move |library| {
            let ids = library
                .tracks_by_tag(
                    &args.key,
                    args.value.as_deref(),
                    args.missing.unwrap_or(false),
                    offset,
                    limit,
                )
                .map_err(failed)?;
            let mut tracks = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(track) = library.track(id).map_err(failed)? {
                    tracks.push(track_out(library, &track, None)?);
                }
            }
            Ok(tracks)
        })
        .await
    }

    #[tool(
        description = "Every label the owner made, with its colour and how many available tracks hold it. Labels are the owner's own names for tracks (moods, purposes), kept in VCMP only, not tags in the files."
    )]
    async fn list_labels(&self) -> Result<String, String> {
        self.read(|library| {
            Ok(library
                .labels()
                .map_err(failed)?
                .iter()
                .map(label_out)
                .collect::<Vec<_>>())
        })
        .await
    }

    #[tool(
        description = "Available tracks holding every one of the labels named, such as all the owner's chill tracks."
    )]
    async fn find_tracks_by_labels(
        &self,
        Parameters(args): Parameters<ByLabelsArgs>,
    ) -> Result<String, String> {
        if args.labels.is_empty() {
            return Err("name at least one label".into());
        }
        let (offset, limit) = page(args.offset, args.limit);
        self.read(move |library| {
            let mut ids = Vec::with_capacity(args.labels.len());
            for name in &args.labels {
                let label = library
                    .label_named(name)
                    .map_err(failed)?
                    .ok_or_else(|| format!("there is no label named {name:?}; see list_labels"))?;
                ids.push(label.id);
            }
            let found = library
                .tracks_with_labels(&ids, offset, limit)
                .map_err(failed)?;
            let mut tracks = Vec::with_capacity(found.len());
            for id in found {
                if let Some(track) = library.track(id).map_err(failed)? {
                    tracks.push(track_out(library, &track, None)?);
                }
            }
            Ok(tracks)
        })
        .await
    }

    #[tool(description = "Every playlist with its id and number of tracks.")]
    async fn list_playlists(&self) -> Result<String, String> {
        self.read(|library| {
            Ok(library
                .playlists()
                .map_err(failed)?
                .iter()
                .map(playlist_out)
                .collect::<Vec<_>>())
        })
        .await
    }

    #[tool(
        description = "One playlist's entries in order, with their positions; an entry whose file is gone stays, marked unavailable."
    )]
    async fn get_playlist(
        &self,
        Parameters(args): Parameters<PlaylistArgs>,
    ) -> Result<String, String> {
        self.read(move |library| {
            let id = PlaylistId(args.playlist_id);
            let playlist = library
                .playlist(id)
                .map_err(failed)?
                .ok_or_else(|| format!("there is no playlist {}", args.playlist_id))?;
            let entries = library.playlist_entries(id).map_err(failed)?;
            Ok(serde_json::json!({
                "playlist": playlist_out(&playlist),
                "entries": entries.iter().map(|entry| Ok(serde_json::json!({
                    "position": entry.position,
                    "track": track_out(library, &entry.track, None)?,
                }))).collect::<Result<Vec<_>, String>>()?,
            }))
        })
        .await
    }

    #[tool(
        description = "Ask to change tags of up to 500 tracks. Nothing is written until the owner reviews and accepts the change in VCMP; the answer is the change set's id, to follow with get_change_set. A file whose tags change before the owner accepts is left alone."
    )]
    async fn propose_tag_changes(
        &self,
        Parameters(args): Parameters<ProposeArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        if args.changes.is_empty() {
            return Err("name at least one track to change".into());
        }
        if args.changes.len() > MAX_FILES {
            return Err(format!(
                "one change covers at most {MAX_FILES} tracks, so the owner can review it; split it"
            ));
        }
        // Each image once, however many tracks it goes into.
        let sources: Vec<CoverSource> = args
            .changes
            .iter()
            .filter_map(|change| change.cover.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let covers = if sources.is_empty() {
            BTreeMap::new()
        } else {
            tokio::task::spawn_blocking(move || load_covers(sources))
                .await
                .map_err(|error| error.to_string())??
        };
        let edits = args
            .changes
            .into_iter()
            .map(|change| edit(change, &covers))
            .collect::<Result<Vec<_>, String>>()?;
        let agent = agent(&context);
        let files = edits.len();
        self.change(move |changes| {
            let id = changes.submit_tags(&agent, edits)?;
            Ok(serde_json::json!({
                "change_set_id": id.0,
                "status": "pending",
                "files": files,
                "note": "Waiting for the owner's review in VCMP.",
            }))
        })
        .await
    }

    #[tool(
        description = "Ask to put labels on tracks and take them off, creating new labels if needed, for up to 500 tracks. Nothing changes until the owner reviews and accepts it in VCMP; the answer is the change set's id, to follow with get_change_set. A track whose labels change before the owner accepts is left alone."
    )]
    async fn propose_label_changes(
        &self,
        Parameters(args): Parameters<ProposeLabelsArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let mut order: Vec<TrackId> = Vec::new();
        let mut by_track: HashMap<TrackId, LabelEdit> = HashMap::new();
        for change in args.changes {
            if change.add.is_empty() && change.remove.is_empty() {
                return Err("each change needs labels to add or remove".into());
            }
            for id in change.track_ids {
                let edit = by_track.entry(TrackId(id)).or_insert_with(|| {
                    order.push(TrackId(id));
                    LabelEdit::default()
                });
                edit.add.extend(change.add.iter().cloned());
                edit.remove.extend(change.remove.iter().cloned());
            }
        }
        if order.is_empty() {
            return Err("name at least one track to change".into());
        }
        if order.len() > MAX_FILES {
            return Err(format!(
                "one change covers at most {MAX_FILES} tracks, so the owner can review it; split it"
            ));
        }
        let new_labels = args
            .new_labels
            .into_iter()
            .map(|label| {
                Ok(NewLabel {
                    name: label.name,
                    colour: colour(&label.colour)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let edits: Vec<(TrackId, LabelEdit)> = order
            .into_iter()
            .map(|id| {
                let edit = by_track.remove(&id).unwrap_or_default();
                (id, edit)
            })
            .collect();
        let agent = agent(&context);
        self.change(move |changes| {
            let id = changes.submit_labels(&agent, edits, new_labels)?;
            Ok(serde_json::json!({
                "change_set_id": id.0,
                "status": "pending",
                "note": "Waiting for the owner's review in VCMP. Tracks that already were as asked are left out.",
            }))
        })
        .await
    }

    #[tool(
        description = "A change set's status and, per file, what it changes: every tag's values before, what was asked, and once written what the file reads back as, with a reason for each file left alone."
    )]
    async fn get_change_set(
        &self,
        Parameters(args): Parameters<ChangeArgs>,
    ) -> Result<String, String> {
        self.read(move |library| {
            let set = library
                .change_set(ChangeSetId(args.change_set_id))
                .map_err(failed)?
                .ok_or_else(|| {
                    format!(
                        "there is no change set {}: it never existed, or it finished more than 30 days ago or the owner deleted its backups",
                        args.change_set_id
                    )
                })?;
            Ok(change_set_out(&set))
        })
        .await
    }

    #[tool(
        description = "The newest change sets, by every agent and the owner's undos, with their status."
    )]
    async fn list_change_sets(
        &self,
        Parameters(args): Parameters<ListChangesArgs>,
    ) -> Result<String, String> {
        let limit = args.limit.unwrap_or(20).clamp(1, MAX_PAGE) as usize;
        self.read(move |library| {
            Ok(library
                .change_sets()
                .map_err(failed)?
                .iter()
                .take(limit)
                .map(summary_out)
                .collect::<Vec<_>>())
        })
        .await
    }

    #[tool(
        description = "Ask to undo an accepted change of tags or labels: it puts back every value the written files (or the labels the tracks) had before; labels the change created go once no track holds them. Like any change, it waits for the owner's review."
    )]
    async fn propose_undo(
        &self,
        Parameters(args): Parameters<ChangeArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let agent = agent(&context);
        self.change(move |changes| {
            let id = changes.submit_undo(&agent, ChangeSetId(args.change_set_id))?;
            Ok(serde_json::json!({ "change_set_id": id.0, "status": "pending" }))
        })
        .await
    }

    #[tool(
        description = "Ask to delete a playlist. The owner is warned that it can't be undone and decides in VCMP; the playlist's tracks stay in the library."
    )]
    async fn propose_playlist_deletion(
        &self,
        Parameters(args): Parameters<PlaylistArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let agent = agent(&context);
        self.change(move |changes| {
            let id = changes.submit_playlist_deletion(&agent, PlaylistId(args.playlist_id))?;
            Ok(serde_json::json!({ "change_set_id": id.0, "status": "pending" }))
        })
        .await
    }

    #[tool(description = "Make an empty playlist; the answer is its id.")]
    async fn create_playlist(
        &self,
        Parameters(args): Parameters<NameArgs>,
    ) -> Result<String, String> {
        if args.name.trim().is_empty() {
            return Err("a playlist needs a name".into());
        }
        self.change(move |changes| {
            let id = changes.edit_playlist(PlaylistEdit::Create(args.name))?;
            Ok(serde_json::json!({ "playlist_id": id.map(|id| id.0) }))
        })
        .await
    }

    #[tool(description = "Rename a playlist.")]
    async fn rename_playlist(
        &self,
        Parameters(args): Parameters<RenameArgs>,
    ) -> Result<String, String> {
        if args.name.trim().is_empty() {
            return Err("a playlist needs a name".into());
        }
        self.change(move |changes| {
            changes.edit_playlist(PlaylistEdit::Rename(
                PlaylistId(args.playlist_id),
                args.name,
            ))?;
            Ok("renamed")
        })
        .await
    }

    #[tool(
        description = "Append tracks to a playlist, in the order given. A track may appear more than once."
    )]
    async fn add_to_playlist(
        &self,
        Parameters(args): Parameters<AddArgs>,
    ) -> Result<String, String> {
        if args.track_ids.len() > MAX_FILES {
            return Err(format!("add at most {MAX_FILES} tracks at a time"));
        }
        self.change(move |changes| {
            let tracks = args.track_ids.into_iter().map(TrackId).collect();
            changes.edit_playlist(PlaylistEdit::Add(PlaylistId(args.playlist_id), tracks))?;
            Ok("added")
        })
        .await
    }

    #[tool(
        description = "Remove entries from a playlist by their positions; the tracks stay in the library."
    )]
    async fn remove_from_playlist(
        &self,
        Parameters(args): Parameters<RemoveArgs>,
    ) -> Result<String, String> {
        self.change(move |changes| {
            changes.edit_playlist(PlaylistEdit::Remove(
                PlaylistId(args.playlist_id),
                args.positions,
            ))?;
            Ok("removed")
        })
        .await
    }

    #[tool(description = "Move one playlist entry to another position.")]
    async fn move_in_playlist(
        &self,
        Parameters(args): Parameters<MoveArgs>,
    ) -> Result<String, String> {
        self.change(move |changes| {
            changes.edit_playlist(PlaylistEdit::Move(
                PlaylistId(args.playlist_id),
                args.from,
                args.to,
            ))?;
            Ok("moved")
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VcmpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("vcmp", env!("CARGO_PKG_VERSION")).with_title("VCMP"),
            )
            .with_instructions(INSTRUCTIONS)
    }
}

/// The largest cover an agent may give, as a file or a download.
const MAX_COVER_BYTES: u64 = 10 * 1000 * 1000;

/// Reads or downloads every cover, each checked to be a JPEG or PNG image.
fn load_covers(sources: Vec<CoverSource>) -> Result<BTreeMap<CoverSource, Vec<u8>>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(concat!("VCMP/", env!("CARGO_PKG_VERSION")))
        .max_redirects(5)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    sources
        .into_iter()
        .map(|source| {
            let data = match &source {
                CoverSource::File(path) => read_cover(path)?,
                CoverSource::Url(url) => download_cover(&agent, url)?,
            };
            if !is_jpeg_or_png(&data) {
                return Err(format!("{}: not a JPEG or PNG image", source_name(&source)));
            }
            Ok((source, data))
        })
        .collect()
}

fn read_cover(path: &str) -> Result<Vec<u8>, String> {
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err(format!(
            "{}: give the cover file's absolute path",
            path.display()
        ));
    }
    let size = std::fs::metadata(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    if size > MAX_COVER_BYTES {
        return Err(format!(
            "{}: a cover may take at most 10 MB",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn download_cover(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!(
            "{url}: only http and https addresses can be downloaded"
        ));
    }
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_COVER_BYTES)
        .read_to_vec()
        .map_err(|error| format!("{url}: {error}"))
}

fn is_jpeg_or_png(data: &[u8]) -> bool {
    data.starts_with(&[0xff, 0xd8, 0xff]) || data.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn source_name(source: &CoverSource) -> &str {
    match source {
        CoverSource::File(path) | CoverSource::Url(path) => path,
    }
}

/// The edit a track change asks for, its cover from `covers`. A field both set and
/// removed is refused.
fn edit(
    change: TrackChange,
    covers: &BTreeMap<CoverSource, Vec<u8>>,
) -> Result<(TrackId, TagEdit), String> {
    let track = change.track_id;
    let removes = |field: Field| change.remove.contains(&field);
    fn field<T>(
        value: Option<T>,
        remove: bool,
        name: &str,
        track: i64,
    ) -> Result<FieldEdit<T>, String> {
        match (value, remove) {
            (Some(_), true) => Err(format!("track {track}: {name} is both set and removed")),
            (Some(value), false) => Ok(FieldEdit::Set(value)),
            (None, true) => Ok(FieldEdit::Clear),
            (None, false) => Ok(FieldEdit::Keep),
        }
    }
    let edit = TagEdit {
        title: field(change.title, removes(Field::Title), "title", track)?,
        artist: field(change.artist, removes(Field::Artist), "artist", track)?,
        album: field(change.album, removes(Field::Album), "album", track)?,
        album_artist: field(
            change.album_artist,
            removes(Field::AlbumArtist),
            "album_artist",
            track,
        )?,
        track_number: field(
            change.track_number,
            removes(Field::TrackNumber),
            "track_number",
            track,
        )?,
        disc_number: field(
            change.disc_number,
            removes(Field::DiscNumber),
            "disc_number",
            track,
        )?,
        year: field(change.year, removes(Field::Year), "year", track)?,
        genre: field(change.genre, removes(Field::Genre), "genre", track)?,
        cover: field(
            change.cover.and_then(|source| covers.get(&source).cloned()),
            removes(Field::Cover),
            "cover",
            track,
        )?,
        other: change
            .tags
            .into_iter()
            .map(|(key, values)| {
                let edit = if values.is_empty() {
                    FieldEdit::Clear
                } else {
                    FieldEdit::Set(values)
                };
                (key, edit)
            })
            .collect(),
    };
    if edit.is_empty() {
        return Err(format!("track {track}: the change changes nothing"));
    }
    Ok((TrackId(track), edit))
}

/// A colour as an agent gives it: a palette name (with or without `palette:`) or
/// `#rrggbb` in either case.
fn colour(given: &str) -> Result<String, String> {
    let given = given.trim().to_lowercase();
    let stored = if LABEL_PALETTE.contains(&given.as_str()) {
        format!("palette:{given}")
    } else {
        given
    };
    if valid_label_colour(&stored) {
        Ok(stored)
    } else {
        Err(format!(
            "{stored:?} is not a label colour: use one of {} or #rrggbb",
            LABEL_PALETTE.join(", ")
        ))
    }
}

fn tracks_out(library: &Library, tracks: &[Track]) -> Result<Vec<serde_json::Value>, String> {
    tracks
        .iter()
        .map(|track| track_out(library, track, None))
        .collect()
}

fn track_out(
    library: &Library,
    track: &Track,
    tags: Option<BTreeMap<String, Vec<String>>>,
) -> Result<serde_json::Value, String> {
    let labels: Vec<String> = library
        .track_labels_of(track.id)
        .map_err(failed)?
        .into_iter()
        .map(|label| label.name)
        .collect();
    let mut out = serde_json::json!({
        "id": track.id.0,
        "title": track.title,
        "artist": track.artist,
        "album": track.album,
        "album_artist": track.album_artist,
        "track_number": track.track_number,
        "disc_number": track.disc_number,
        "year": track.year,
        "genre": track.genre,
        "album_id": track.album_id.map(|id| id.0),
        "artist_id": track.artist_id.map(|id| id.0),
        "duration_seconds": track.duration.as_secs(),
        "format": track.format.map(|format| format!("{format:?}")),
        "path": track.path.to_string_lossy(),
        "available": track.available,
        "labels": labels,
    });
    if let Some(tags) = tags {
        out["tags"] = serde_json::json!(tags);
    }
    Ok(out)
}

fn label_out(label: &Label) -> serde_json::Value {
    serde_json::json!({
        "name": label.name,
        "colour": label.colour.strip_prefix("palette:").unwrap_or(&label.colour),
        "track_count": label.track_count,
    })
}

fn album_out(album: &Album) -> serde_json::Value {
    serde_json::json!({
        "id": album.id.0,
        "title": album.title,
        "artist": album.artist,
        "artist_id": album.artist_id.map(|id| id.0),
        "year": album.year,
        "track_count": album.track_count,
        "musicbrainz_release_id": album.musicbrainz_id,
        "unknown": album.unknown,
        "has_artwork": album.artwork.is_some(),
    })
}

fn artist_out(artist: &Artist) -> serde_json::Value {
    serde_json::json!({
        "id": artist.id.0,
        "name": artist.name,
        "album_count": artist.album_count,
        "track_count": artist.track_count,
    })
}

fn playlist_out(playlist: &Playlist) -> serde_json::Value {
    serde_json::json!({
        "id": playlist.id.0,
        "name": playlist.name,
        "track_count": playlist.track_count,
        "available_count": playlist.available_count,
    })
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn summary_out(summary: &ChangeSetSummary) -> serde_json::Value {
    let (kind, playlist) = match &summary.kind {
        ChangeKind::Tags if summary.undoes.is_some() => ("undo", None),
        ChangeKind::Tags => ("tags", None),
        ChangeKind::Labels if summary.undoes.is_some() => ("label_undo", None),
        ChangeKind::Labels => ("labels", None),
        ChangeKind::PlaylistDeletion { playlist, name } => (
            "playlist_deletion",
            Some(serde_json::json!({ "id": playlist.0, "name": name })),
        ),
    };
    serde_json::json!({
        "change_set_id": summary.id.0,
        "kind": kind,
        "status": status(summary.status),
        "asked_by": summary.agent,
        "undoes": summary.undoes.map(|id| id.0),
        "playlist": playlist,
        "files": summary.file_count,
        "created_unix": seconds(summary.created),
        "finished_unix": summary.finished.map(seconds),
    })
}

fn change_set_out(set: &ChangeSet) -> serde_json::Value {
    let mut out = summary_out(&set.summary);
    if set.summary.kind == ChangeKind::Labels {
        return label_set_out(set, out);
    }
    out["files"] = serde_json::json!(
        set.files
            .iter()
            .map(|file| {
                let (result, detail) = result_out(&file.result);
                serde_json::json!({
                    "track_id": file.track.0,
                    "path": file.path.to_string_lossy(),
                    "asked": file.edits.iter().filter(|(target, _)| target != "cover").map(|(target, values)| {
                        serde_json::json!({ "field": target, "values": values })
                    }).collect::<Vec<_>>(),
                    "cover": file.edits.iter().find(|(target, _)| target == "cover").map(|(_, image)| {
                        if image.is_some() { "replaced" } else { "removed" }
                    }),
                    "tags": file.values.iter().map(|value| serde_json::json!({
                        "key": value.key,
                        "before": value.before,
                        "after": value.after,
                    })).collect::<Vec<_>>(),
                    "result": result,
                    "detail": detail,
                })
            })
            .collect::<Vec<_>>()
    );
    out
}

/// A label set: per track, each label it changes, whether the track held it before,
/// whether it should, and whether it does once accepted.
fn label_set_out(set: &ChangeSet, mut out: serde_json::Value) -> serde_json::Value {
    out["new_labels"] = serde_json::json!(
        set.new_labels
            .iter()
            .map(|label| serde_json::json!({
                "name": label.name,
                "colour": label.colour.strip_prefix("palette:").unwrap_or(&label.colour),
                "created": label.created,
            }))
            .collect::<Vec<_>>()
    );
    out["tracks"] = serde_json::json!(
        set.files
            .iter()
            .map(|file| {
                let (result, detail) = result_out(&file.result);
                serde_json::json!({
                    "track_id": file.track.0,
                    "labels": file.values.iter().map(|value| {
                        let asked = file
                            .edits
                            .iter()
                            .find(|(target, _)| *target == value.key)
                            .is_some_and(|(_, values)| values.is_some());
                        serde_json::json!({
                            "label": value.key.strip_prefix("label:").unwrap_or(&value.key),
                            "held_before": !value.before.is_empty(),
                            "asked": asked,
                            "held_after": value.after.as_ref().map(|after| !after.is_empty()),
                        })
                    }).collect::<Vec<_>>(),
                    "result": result,
                    "detail": detail,
                })
            })
            .collect::<Vec<_>>()
    );
    out.as_object_mut().map(|object| object.remove("files"));
    out["tracks_changed"] = serde_json::json!(set.summary.file_count);
    out
}

fn result_out(result: &Option<FileResult>) -> (Option<&'static str>, Option<String>) {
    match result {
        None => (None, None),
        Some(FileResult::Written) => (Some("written"), None),
        Some(FileResult::Conflict(detail)) => (Some("conflict"), Some(detail.clone())),
        Some(FileResult::Failed(detail)) => (Some("failed"), Some(detail.clone())),
    }
}

fn status(status: ChangeStatus) -> &'static str {
    match status {
        ChangeStatus::Pending => "pending",
        ChangeStatus::Accepted => "accepted",
        ChangeStatus::PartlyWritten => "partly_written",
        ChangeStatus::Rejected => "rejected",
        ChangeStatus::Discarded => "discarded",
        ChangeStatus::Undone => "undone",
    }
}
