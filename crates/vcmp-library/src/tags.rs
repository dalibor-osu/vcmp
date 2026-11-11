//! Reading a music file's tags and audio properties. Files are only ever opened for
//! reading here.
//!
//! Besides the fields VCMP shows, a read lists every text item of the tag VCMP reads
//! as [`TagEntry`] values, so the database holds what the file says in full.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use lofty::config::ParseOptions;
use lofty::error::FileParseError;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4Codec, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::VorbisFile;
use lofty::ogg::tag::VorbisComments;
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, ItemValue, SplitTag, Tag, TagType};

/// Audio formats VCMP plays. AAC and ALAC are the two codecs VCMP plays in an MP4
/// (`.m4a`) file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Mp3,
    Flac,
    OggVorbis,
    Wav,
    Aiff,
    Aac,
    Alac,
}

impl Format {
    const ALL: [Self; 7] = [
        Self::Mp3,
        Self::Flac,
        Self::OggVorbis,
        Self::Wav,
        Self::Aiff,
        Self::Aac,
        Self::Alac,
    ];

    /// Whether the scanner looks at a file with this name. Which format it holds, if
    /// any VCMP plays, is known only once the file is read: an `.m4a` file may hold
    /// AAC or ALAC.
    pub(crate) fn is_candidate(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "mp3" | "flac" | "ogg" | "oga" | "wav" | "aif" | "aiff" | "aifc" | "m4a"
                )
            })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::OggVorbis => "ogg-vorbis",
            Self::Wav => "wav",
            Self::Aiff => "aiff",
            Self::Aac => "aac",
            Self::Alac => "alac",
        }
    }

    /// The audio is stored without loss: FLAC, WAV, AIFF and ALAC, not MP3, Vorbis or
    /// AAC.
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Wav | Self::Aiff | Self::Alac)
    }

    /// The name people know the format by, such as "FLAC".
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::OggVorbis => "Ogg Vorbis",
            Self::Wav => "WAV",
            Self::Aiff => "AIFF",
            Self::Aac => "AAC",
            Self::Alac => "ALAC",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.as_str() == text)
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
    /// frame has one (`id3:TXXX:MY FIELD`, `id3:COMM:iTunNORM`), `vorbis:` and the
    /// field name in capitals (`vorbis:MYFIELD`), or `mp4:` and the atom's four
    /// characters or, for a freeform atom, `----`, its mean and its name
    /// (`mp4:stik`, `mp4:----:com.apple.iTunes:MY FIELD`).
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
    // Each format is read as its own type rather than through the probe: only the
    // format's own tag still holds the items lofty has no common name for.
    let mut reader = probe.into_inner();
    let (format, file, native_entries): (Format, TaggedFile, Option<Vec<TagEntry>>) =
        match file_type {
            FileType::Mpeg => {
                let file = MpegFile::read_from(&mut reader, options).map_err(unreadable)?;
                let entries = file.id3v2().map(id3v2_entries);
                (Format::Mp3, file.into(), entries)
            }
            FileType::Wav => {
                let file = WavFile::read_from(&mut reader, options).map_err(unreadable)?;
                let entries = file.id3v2().map(id3v2_entries);
                (Format::Wav, file.into(), entries)
            }
            FileType::Aiff => {
                let file = AiffFile::read_from(&mut reader, options).map_err(unreadable)?;
                let entries = file.id3v2().map(id3v2_entries);
                (Format::Aiff, file.into(), entries)
            }
            FileType::Flac => {
                let file = FlacFile::read_from(&mut reader, options).map_err(unreadable)?;
                let entries = file.vorbis_comments().map(vorbis_entries);
                (Format::Flac, file.into(), entries)
            }
            FileType::Vorbis => {
                let file = VorbisFile::read_from(&mut reader, options).map_err(unreadable)?;
                let entries = Some(vorbis_entries(file.vorbis_comments()));
                (Format::OggVorbis, file.into(), entries)
            }
            FileType::Mp4 => {
                let file = Mp4File::read_from(&mut reader, options).map_err(unreadable)?;
                let format = match file.properties().codec() {
                    Some(Mp4Codec::AAC) => Format::Aac,
                    Some(Mp4Codec::ALAC) => Format::Alac,
                    codec => {
                        return Err(format!("unsupported audio format (MP4 holding {codec:?})"));
                    }
                };
                if mp4_holds_video(&mut reader).map_err(|error| format!("unreadable: {error}"))? {
                    return Err("unsupported audio format (MP4 holding video)".to_owned());
                }
                let entries = file.ilst().map(mp4_entries);
                (format, file.into(), entries)
            }
            other => return Err(format!("unsupported audio format ({other:?})")),
        };
    let duration = file.properties().duration();
    // The native chunks of WAV (RIFF INFO) and AIFF (text chunks) hold few fields, so
    // an ID3v2 tag wins when present.
    let preferred = match format {
        Format::Wav | Format::Aiff => file.tag(TagType::Id3v2).or_else(|| file.primary_tag()),
        _ => file.primary_tag(),
    }
    .or_else(|| file.first_tag());
    let tags = preferred.map(tags_from).unwrap_or_default();
    // Without the tag VCMP writes (an MP3 with only ID3v1 or APE, a WAV with only RIFF
    // INFO, an AIFF with only text chunks), the entries come from the tag the fields
    // were read from.
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

pub(crate) fn mp4_entries(tag: &Ilst) -> Vec<TagEntry> {
    // As with ID3v2, lofty's split takes the atoms it has common names for, and the
    // rest become `mp4:` entries.
    let (remainder, generic) = tag.clone().split_tag();
    let mut entries = generic_entries(&generic);
    for atom in &*remainder {
        let key = mp4_atom_key(atom.ident());
        for data in atom.data() {
            if let Some(value) = atom_text(data) {
                entries.push(TagEntry {
                    key: key.clone(),
                    value,
                });
            }
        }
    }
    entries
}

/// The key the scan lists an atom's values under: lofty's common name when it has one
/// (a number lofty's split leaves in its atom, like the integer `tmpo`, is still the
/// `IntegerBpm`), otherwise the atom's own key.
pub(crate) fn mp4_atom_key(ident: &AtomIdent<'_>) -> String {
    match ItemKey::from_key(TagType::Mp4Ilst, &atom_name(ident)) {
        Some(item) => generic_key(item),
        None => mp4_key(ident),
    }
}

/// `mp4:` and the atom's name.
pub(crate) fn mp4_key(ident: &AtomIdent<'_>) -> String {
    format!("mp4:{}", atom_name(ident))
}

/// An atom's name as lofty spells it: its four characters, the bytes read as
/// Latin-1 (`0xA9` is the copyright sign), or `----:<mean>:<name>`.
fn atom_name(ident: &AtomIdent<'_>) -> String {
    match ident {
        AtomIdent::Fourcc(fourcc) => fourcc.iter().copied().map(char::from).collect(),
        AtomIdent::Freeform { mean, name } => format!("----:{mean}:{name}"),
    }
}

/// An atom value as text: text as it is, numbers and flags in decimal. Pictures and
/// other binary data hold no text to list.
pub(crate) fn atom_text(data: &AtomData) -> Option<String> {
    match data {
        AtomData::UTF8(text) | AtomData::UTF16(text) => Some(text.clone()),
        AtomData::SignedInteger(number) => Some(number.to_string()),
        AtomData::UnsignedInteger(number) => Some(number.to_string()),
        AtomData::Bool(flag) => Some(u8::from(*flag).to_string()),
        AtomData::Picture(_) | AtomData::Unknown { .. } => None,
    }
}

/// The keys the scan lists an atom's values under.
pub(crate) fn atom_keys(atom: &Atom<'static>) -> Vec<String> {
    let mut single = Ilst::new();
    single.insert(atom.clone());
    mp4_entries(&single)
        .into_iter()
        .map(|entry| entry.key)
        .collect()
}

/// Whether an MP4 file has a video track, by the handler of each track
/// (`moov/trak/mdia/hdlr`). lofty reads only the audio track.
fn mp4_holds_video<R: Read + Seek>(reader: &mut R) -> io::Result<bool> {
    let end = reader.seek(SeekFrom::End(0))?;
    let Some(moov) = child_box(reader, (0, end), *b"moov")? else {
        return Ok(false);
    };
    let mut position = moov.0;
    while let Some((kind, body, next)) = next_box(reader, position, moov.1)? {
        position = next;
        if kind != *b"trak" {
            continue;
        }
        let Some(mdia) = child_box(reader, (body, next), *b"mdia")? else {
            continue;
        };
        let Some((hdlr, hdlr_end)) = child_box(reader, mdia, *b"hdlr")? else {
            continue;
        };
        // Version and flags, a reserved word, then the handler type.
        if hdlr_end - hdlr < 12 {
            continue;
        }
        reader.seek(SeekFrom::Start(hdlr + 8))?;
        let mut handler = [0; 4];
        reader.read_exact(&mut handler)?;
        if handler == *b"vide" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The box at `position`, ending by `end`: its type, where its body starts and where
/// it ends. None past the last box, or at a box that doesn't fit.
fn next_box<R: Read + Seek>(
    reader: &mut R,
    position: u64,
    end: u64,
) -> io::Result<Option<([u8; 4], u64, u64)>> {
    if end.saturating_sub(position) < 8 {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(position))?;
    let mut header = [0; 8];
    reader.read_exact(&mut header)?;
    let kind = [header[4], header[5], header[6], header[7]];
    let (body, size) = match u32::from_be_bytes([header[0], header[1], header[2], header[3]]) {
        // The box runs to the end of its parent.
        0 => (position + 8, end - position),
        // A 64-bit size follows the type.
        1 => {
            if end - position < 16 {
                return Ok(None);
            }
            let mut size = [0; 8];
            reader.read_exact(&mut size)?;
            (position + 16, u64::from_be_bytes(size))
        }
        size => (position + 8, u64::from(size)),
    };
    Ok(position
        .checked_add(size)
        .filter(|&box_end| box_end <= end && body <= box_end)
        .map(|box_end| (kind, body, box_end)))
}

/// The body of the first box of type `wanted` among the boxes in `within`.
fn child_box<R: Read + Seek>(
    reader: &mut R,
    within: (u64, u64),
    wanted: [u8; 4],
) -> io::Result<Option<(u64, u64)>> {
    let mut position = within.0;
    while let Some((kind, body, box_end)) = next_box(reader, position, within.1)? {
        if kind == wanted {
            return Ok(Some((body, box_end)));
        }
        position = box_end;
    }
    Ok(None)
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
