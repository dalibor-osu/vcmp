//! Reading a music file's tags and audio properties. Files are only ever opened for
//! reading here.

use std::path::Path;
use std::time::Duration;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagType};

/// Audio formats VCMP plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Mp3,
    Flac,
    OggVorbis,
    Wav,
}

impl Format {
    pub(crate) fn from_extension(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "mp3" => Some(Self::Mp3),
            "flac" => Some(Self::Flac),
            "ogg" | "oga" => Some(Self::OggVorbis),
            "wav" => Some(Self::Wav),
            _ => None,
        }
    }

    fn from_file_type(file_type: FileType) -> Option<Self> {
        match file_type {
            FileType::Mpeg => Some(Self::Mp3),
            FileType::Flac => Some(Self::Flac),
            FileType::Vorbis => Some(Self::OggVorbis),
            FileType::Wav => Some(Self::Wav),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::OggVorbis => "ogg-vorbis",
            Self::Wav => "wav",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        [Self::Mp3, Self::Flac, Self::OggVorbis, Self::Wav]
            .into_iter()
            .find(|format| format.as_str() == text)
    }
}

#[derive(Debug, Default)]
pub(crate) struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub mb_album_id: Option<String>,
    pub mb_recording_id: Option<String>,
    pub mb_artist_id: Option<String>,
    pub mb_album_artist_id: Option<String>,
}

pub(crate) struct ReadTrack {
    pub format: Format,
    pub duration: Duration,
    pub tags: TrackTags,
    /// The embedded cover, when asked for and present.
    pub cover: Option<Vec<u8>>,
}

/// Reads the tags and duration, plus the embedded cover when `with_cover` is set. The
/// cover sits next to the tags, so reading it here costs no extra disk seek. The
/// error is a reason suitable for the scan report.
pub(crate) fn read(path: &Path, with_cover: bool) -> Result<ReadTrack, String> {
    let file = Probe::open(path)
        .and_then(|probe| {
            Ok(probe
                .options(ParseOptions::new().read_cover_art(with_cover))
                .guess_file_type()?)
        })
        .and_then(|probe| probe.read())
        .map_err(|error| format!("unreadable: {error}"))?;
    let format = Format::from_file_type(file.file_type())
        .ok_or_else(|| format!("unsupported audio format ({:?})", file.file_type()))?;
    let duration = file.properties().duration();
    // WAV's native RIFF INFO chunk holds few fields, so an ID3v2 tag wins when present.
    let preferred = match format {
        Format::Wav => file.tag(TagType::Id3v2).or_else(|| file.primary_tag()),
        _ => file.primary_tag(),
    };
    let tags = preferred
        .or_else(|| file.first_tag())
        .map(tags_from)
        .unwrap_or_default();
    let cover = if with_cover { front_cover(&file) } else { None };
    Ok(ReadTrack {
        format,
        duration,
        tags,
        cover,
    })
}

/// The embedded cover of a file, read on its own.
pub(crate) fn embedded_cover(path: &Path) -> Option<Vec<u8>> {
    let file = Probe::open(path)
        .ok()?
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    front_cover(&file)
}

/// The front cover, or failing that the first picture.
fn front_cover(file: &TaggedFile) -> Option<Vec<u8>> {
    let pictures = file
        .tags()
        .iter()
        .flat_map(|tag| tag.pictures())
        .collect::<Vec<_>>();
    pictures
        .iter()
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())
        .map(|picture| picture.data().to_vec())
}

fn tags_from(tag: &Tag) -> TrackTags {
    let text = |key: ItemKey| {
        tag.get_string(key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    // Multi-valued artist fields (Vorbis comments allow several) are joined for display.
    let joined = |key: ItemKey| {
        let values: Vec<&str> = tag
            .get_strings(key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        (!values.is_empty()).then(|| values.join(", "))
    };
    let year = tag
        .date()
        .map(|date| i32::from(date.year))
        .or_else(|| text(ItemKey::Year).and_then(|year| year.get(..4)?.parse().ok()));

    TrackTags {
        title: tag
            .title()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        artist: joined(ItemKey::TrackArtist),
        album: tag
            .album()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        album_artist: joined(ItemKey::AlbumArtist),
        track_number: tag.track(),
        disc_number: tag.disk(),
        year,
        genre: tag
            .genre()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        mb_album_id: text(ItemKey::MusicBrainzReleaseId),
        mb_recording_id: text(ItemKey::MusicBrainzRecordingId),
        mb_artist_id: text(ItemKey::MusicBrainzArtistId),
        mb_album_artist_id: text(ItemKey::MusicBrainzReleaseArtistId),
    }
}
