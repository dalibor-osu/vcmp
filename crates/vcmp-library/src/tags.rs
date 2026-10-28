//! Reading a music file's tags and audio properties. Files are only ever opened for
//! reading here.
//!
//! Besides the fields VCMP shows, a read lists every text item of the tag VCMP reads
//! as [`TagEntry`] values, so the database holds what the file says in full.

use std::path::Path;
use std::time::Duration;

use lofty::config::ParseOptions;
use lofty::error::FileParseError;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::iff::wav::WavFile;
use lofty::mpeg::MpegFile;
use lofty::ogg::VorbisFile;
use lofty::ogg::tag::VorbisComments;
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, ItemValue, SplitTag, Tag, TagType};

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

    /// The audio is stored without loss: FLAC and WAV, not MP3 or Vorbis.
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Wav)
    }

    /// The name people know the format by, such as "FLAC".
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::OggVorbis => "Ogg Vorbis",
            Self::Wav => "WAV",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        [Self::Mp3, Self::Flac, Self::OggVorbis, Self::Wav]
            .into_iter()
            .find(|format| format.as_str() == text)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

/// One text item of the tag VCMP reads, the way the database stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntry {
    /// lofty's format-independent name when the item has one (`Composer`,
    /// `MusicBrainzReleaseId`), otherwise the format's own key behind its prefix:
    /// `id3:` and the frame ID, with the description after another colon where the
    /// frame has one (`id3:TXXX:MY FIELD`, `id3:COMM:iTunNORM`), or `vorbis:` and
    /// the field name in capitals (`vorbis:MYFIELD`).
    pub key: String,
    /// The value as the file holds it, untrimmed. A field with several values gives
    /// one entry per value.
    pub value: String,
}

pub(crate) struct ReadTrack {
    pub format: Format,
    pub duration: Duration,
    pub tags: TrackTags,
    /// Every text item of the tag the fields above come from. Items lofty has a
    /// common name for come first, then the rest in file order.
    pub entries: Vec<TagEntry>,
    /// The embedded cover, when asked for and present.
    pub cover: Option<Vec<u8>>,
}

/// Reads the tags and duration, plus the embedded cover when `with_cover` is set. The
/// cover sits next to the tags, so reading it here costs no extra disk seek. The
/// error is a reason suitable for the scan report.
pub(crate) fn read(path: &Path, with_cover: bool) -> Result<ReadTrack, String> {
    let unreadable = |error: FileParseError| format!("unreadable: {error}");
    let options = ParseOptions::new().read_cover_art(with_cover);
    let probe = Probe::open(path)
        .and_then(|probe| Ok(probe.options(options).guess_file_type()?))
        .map_err(unreadable)?;
    let file_type = probe
        .file_type()
        .ok_or_else(|| "unreadable: the audio format could not be determined".to_owned())?;
    let format = Format::from_file_type(file_type)
        .ok_or_else(|| format!("unsupported audio format ({file_type:?})"))?;
    // Each format is read as its own type rather than through the probe: only the
    // format's own tag still holds the items lofty has no common name for.
    let mut reader = probe.into_inner();
    let (file, native_entries): (TaggedFile, Option<Vec<TagEntry>>) = match format {
        Format::Mp3 => {
            let file = MpegFile::read_from(&mut reader, options).map_err(unreadable)?;
            let entries = file.id3v2().map(id3v2_entries);
            (file.into(), entries)
        }
        Format::Wav => {
            let file = WavFile::read_from(&mut reader, options).map_err(unreadable)?;
            let entries = file.id3v2().map(id3v2_entries);
            (file.into(), entries)
        }
        Format::Flac => {
            let file = FlacFile::read_from(&mut reader, options).map_err(unreadable)?;
            let entries = file.vorbis_comments().map(vorbis_entries);
            (file.into(), entries)
        }
        Format::OggVorbis => {
            let file = VorbisFile::read_from(&mut reader, options).map_err(unreadable)?;
            let entries = Some(vorbis_entries(file.vorbis_comments()));
            (file.into(), entries)
        }
    };
    let duration = file.properties().duration();
    // WAV's native RIFF INFO chunk holds few fields, so an ID3v2 tag wins when present.
    let preferred = match format {
        Format::Wav => file.tag(TagType::Id3v2).or_else(|| file.primary_tag()),
        _ => file.primary_tag(),
    }
    .or_else(|| file.first_tag());
    let tags = preferred.map(tags_from).unwrap_or_default();
    // Without the tag VCMP writes (an MP3 with only ID3v1 or APE, a WAV with only RIFF
    // INFO), the entries come from the tag the fields were read from.
    let entries = native_entries
        .or_else(|| preferred.map(generic_entries))
        .unwrap_or_default();
    let cover = if with_cover { front_cover(&file) } else { None };
    Ok(ReadTrack {
        format,
        duration,
        tags,
        entries,
        cover,
    })
}

/// The name a [`TagEntry`] uses for an item lofty knows across formats.
pub(crate) fn generic_key(key: ItemKey) -> String {
    format!("{key:?}")
}

fn item_text(value: &ItemValue) -> Option<&str> {
    match value {
        ItemValue::Text(text) | ItemValue::Locator(text) => Some(text),
        ItemValue::Binary(_) => None,
    }
}

fn generic_entries(tag: &Tag) -> Vec<TagEntry> {
    tag.items()
        .filter_map(|item| {
            Some(TagEntry {
                key: generic_key(item.key()),
                value: item_text(item.value())?.to_owned(),
            })
        })
        .collect()
}

/// ID3v2.4 text frames separate several values with a NUL.
const ID3V2_SEPARATOR: char = '\0';

pub(crate) fn id3v2_entries(tag: &Id3v2Tag) -> Vec<TagEntry> {
    // lofty's split moves the frames it has common names for into a generic tag and
    // keeps the rest, which become `id3:` entries.
    let (remainder, generic) = tag.clone().split_tag();
    let mut entries = Vec::new();
    for item in generic.items() {
        let Some(value) = item_text(item.value()) else {
            continue;
        };
        // A comment or lyrics frame with a description ("iTunNORM") is not the plain
        // comment, so it keeps its frame ID and description.
        let key = match item.key().map_key(TagType::Id3v2) {
            Some(id) if !item.description().is_empty() => described(id, item.description()),
            _ => generic_key(item.key()),
        };
        entries.push(TagEntry {
            key,
            value: value.to_owned(),
        });
    }
    let mut push = |key: String, values: &str| {
        for value in values.split(ID3V2_SEPARATOR) {
            entries.push(TagEntry {
                key: key.clone(),
                value: value.to_owned(),
            });
        }
    };
    for frame in &*remainder {
        let id = frame.id_str();
        match frame {
            Frame::Text(frame) => push(format!("id3:{id}"), &frame.value),
            Frame::Url(frame) => push(format!("id3:{id}"), frame.url()),
            Frame::Timestamp(frame) => push(format!("id3:{id}"), &frame.timestamp.to_string()),
            Frame::UserText(frame) => push(described(id, &frame.description), &frame.content),
            Frame::UserUrl(frame) => push(described(id, &frame.description), &frame.content),
            Frame::Comment(frame) => push(described(id, &frame.description), &frame.content),
            Frame::UnsynchronizedText(frame) => {
                push(described(id, &frame.description), &frame.content)
            }
            Frame::Popularimeter(frame) => {
                push(described(id, &frame.email), &frame.rating.to_string())
            }
            Frame::KeyValue(frame) => {
                for (role, name) in frame.key_value_pairs.iter() {
                    push(described(id, role), name);
                }
            }
            // Pictures, binary data, chapters and the like hold no text to list.
            _ => {}
        }
    }
    entries
}

/// `id3:` and the frame ID, plus the frame's description when it has one.
fn described(id: &str, description: &str) -> String {
    if description.is_empty() {
        format!("id3:{id}")
    } else {
        format!("id3:{id}:{description}")
    }
}

pub(crate) fn vorbis_entries(tag: &VorbisComments) -> Vec<TagEntry> {
    tag.items()
        .map(|(key, value)| TagEntry {
            key: ItemKey::from_key(TagType::VorbisComments, key)
                .map(generic_key)
                .unwrap_or_else(|| format!("vorbis:{}", key.to_ascii_uppercase())),
            value: value.to_owned(),
        })
        .collect()
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
