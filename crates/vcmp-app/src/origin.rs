use std::path::PathBuf;

use vcmp_library::{AlbumId, ArtistId, LabelId, PlaylistId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Album(AlbumId),
    Artist(ArtistId),
    Playlist(PlaylistId),
    Label(LabelId),
    Folder(PathBuf),
    Agent(String),
    Search,
    Songs,
    Favourites,
}

impl Origin {
    pub fn key(&self) -> String {
        match self {
            Self::Album(id) => format!("album:{}", id.0),
            Self::Artist(id) => format!("artist:{}", id.0),
            Self::Playlist(id) => format!("playlist:{}", id.0),
            Self::Label(id) => format!("label:{}", id.0),
            Self::Folder(path) => format!("folder:{}", path.to_string_lossy()),
            Self::Agent(name) => format!("agent:{name}"),
            Self::Search => "search".into(),
            Self::Songs => "songs".into(),
            Self::Favourites => "favourites".into(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "search" => return Some(Self::Search),
            "songs" => return Some(Self::Songs),
            "favourites" => return Some(Self::Favourites),
            _ => {}
        }
        let (kind, rest) = key.split_once(':')?;
        let id = || rest.parse::<i64>().ok();
        Some(match kind {
            "album" => Self::Album(AlbumId(id()?)),
            "artist" => Self::Artist(ArtistId(id()?)),
            "playlist" => Self::Playlist(PlaylistId(id()?)),
            "label" => Self::Label(LabelId(id()?)),
            "folder" => Self::Folder(PathBuf::from(rest)),
            "agent" => Self::Agent(rest.to_owned()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_the_form_saved_queues_and_play_history_hold() {
        let origins = [
            (Origin::Album(AlbumId(7)), "album:7"),
            (Origin::Artist(ArtistId(3)), "artist:3"),
            (Origin::Playlist(PlaylistId(12)), "playlist:12"),
            (Origin::Label(LabelId(5)), "label:5"),
            (
                Origin::Folder(PathBuf::from("/music/a:b")),
                "folder:/music/a:b",
            ),
            (Origin::Agent("Claude Code".into()), "agent:Claude Code"),
            (Origin::Search, "search"),
            (Origin::Songs, "songs"),
            (Origin::Favourites, "favourites"),
        ];
        for (origin, key) in origins {
            assert_eq!(origin.key(), key);
            assert_eq!(Origin::from_key(key), Some(origin));
        }
        for unknown in ["", "album:x", "nothing:1", "album"] {
            assert_eq!(Origin::from_key(unknown), None, "{unknown}");
        }
    }
}
