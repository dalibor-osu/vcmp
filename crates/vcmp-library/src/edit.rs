//! Writing tags back to music files.
//!
//! Every write goes through a copy. The file is copied next to itself, the copy is
//! edited and read back, and only a copy whose every field checks out replaces the
//! original, by an atomic rename. A failure at any step leaves the original as it was.
//!
//! Only the tag VCMP reads is written: ID3v2 in MP3 and WAV files, Vorbis comments in
//! FLAC and Ogg Vorbis files. It is edited in place, so frames and fields the editor
//! does not know are kept as they are. Tags beyond the nine fields go through
//! [`crate::other_tags`].

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Seek};
use std::path::Path;

use lofty::TextEncoding;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::id3::v2::{
    Frame, FrameId, Id3v2Tag, Id3v2Version, TextInformationFrame, TimestampFrame,
};
use lofty::iff::wav::WavFile;
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureInformation, PictureType};
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};

use crate::other_tags::{self, KeyProblem, Resolved};
use crate::tags::{self, Format, TagEntry, TrackTags};

/// A change to one field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FieldEdit<T> {
    /// Leave the field as the file has it.
    #[default]
    Keep,
    Set(T),
    /// Remove the field from the file.
    Clear,
}

impl FieldEdit<String> {
    /// Surrounding whitespace is dropped, and an empty value clears the field.
    fn normalized(&self) -> FieldEdit<&str> {
        match self {
            Self::Keep => FieldEdit::Keep,
            Self::Clear => FieldEdit::Clear,
            Self::Set(value) => match value.trim() {
                "" => FieldEdit::Clear,
                value => FieldEdit::Set(value),
            },
        }
    }
}

impl<T> FieldEdit<T> {
    pub(crate) fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

/// The changes to write into a file. Fields left at `Keep` are not touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagEdit {
    pub title: FieldEdit<String>,
    pub artist: FieldEdit<String>,
    pub album: FieldEdit<String>,
    pub album_artist: FieldEdit<String>,
    pub track_number: FieldEdit<u32>,
    pub disc_number: FieldEdit<u32>,
    pub year: FieldEdit<i32>,
    pub genre: FieldEdit<String>,
    /// The embedded front cover, as JPEG or PNG bytes.
    pub cover: FieldEdit<Vec<u8>>,
    /// Every other tag, by the key [`crate::Library::track_tags`] lists it under:
    /// replaced by the values given, in order, or removed. Values are written exactly
    /// as given, untrimmed; an empty list removes the tag. The keys of the fields
    /// above are refused here.
    pub other: BTreeMap<String, FieldEdit<Vec<String>>>,
}

/// The fields a [`TagEdit`] can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagField {
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

/// Why a [`TagEdit`] cannot be written to any file.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidTagEdit {
    #[error("{0:?} contains a control character")]
    ControlCharacter(TagField),
    #[error("the year must be between 1 and 9999")]
    YearOutOfRange,
    #[error("the cover must be a JPEG or PNG image")]
    CoverFormat,
    #[error("{0:?} is not a tag key")]
    UnknownKey(String),
    #[error("{0:?} is one of the fields VCMP shows; edit it through that field")]
    ShownField(String),
    #[error("VCMP can't write values under {0:?}")]
    UnwritableKey(String),
    #[error("a value of {0:?} contains a control character other than a line break or tab")]
    ValueCharacter(String),
    #[error("a value of {0:?} is empty")]
    EmptyValue(String),
    #[error("{0:?} takes whole numbers")]
    NotANumber(String),
    /// lofty drops the value in every kind of tag: it lacks the form the key takes,
    /// like a rating that isn't `email|stars|count`.
    #[error("a value of {0:?} doesn't have the form the tag takes")]
    BadValue(String),
}

impl TagEdit {
    /// Whether the edit changes nothing.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Checks the values before any file is touched. [`crate::Library::write_tags`]
    /// does this too; the editor can call it to point at the wrong field early.
    pub fn validate(&self) -> Result<(), InvalidTagEdit> {
        let texts = [
            (TagField::Title, &self.title),
            (TagField::Artist, &self.artist),
            (TagField::Album, &self.album),
            (TagField::AlbumArtist, &self.album_artist),
            (TagField::Genre, &self.genre),
        ];
        for (field, edit) in texts {
            // A NUL would split an ID3v2 value in two; line breaks don't belong in
            // these one-line fields.
            if let FieldEdit::Set(value) = edit
                && value.chars().any(char::is_control)
            {
                return Err(InvalidTagEdit::ControlCharacter(field));
            }
        }
        if let FieldEdit::Set(year) = self.year
            && !(1..=9999).contains(&year)
        {
            return Err(InvalidTagEdit::YearOutOfRange);
        }
        if let FieldEdit::Set(data) = &self.cover {
            cover_mime_type(data).ok_or(InvalidTagEdit::CoverFormat)?;
        }
        for (key, edit) in &self.other {
            let values = match edit {
                FieldEdit::Set(values) => values.as_slice(),
                _ => &[],
            };
            for value in values {
                // Lyrics and notes span lines; a NUL would split an ID3v2 value.
                if value
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                {
                    return Err(InvalidTagEdit::ValueCharacter(key.clone()));
                }
                // lofty drops a total or movement number that isn't a whole number.
                if other_tags::takes_number(key) && !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(InvalidTagEdit::NotANumber(key.clone()));
                }
                if value.is_empty() {
                    return Err(InvalidTagEdit::EmptyValue(key.clone()));
                }
            }
            other_tags::check_key(key, values).map_err(|problem| match problem {
                KeyProblem::Unknown => InvalidTagEdit::UnknownKey(key.clone()),
                KeyProblem::Shown => InvalidTagEdit::ShownField(key.clone()),
                KeyProblem::Unwritable => InvalidTagEdit::UnwritableKey(key.clone()),
                KeyProblem::BadValue => InvalidTagEdit::BadValue(key.clone()),
            })?;
        }
        Ok(())
    }

    /// What the scanner should read from the file once `self`, its other tags resolved
    /// as `other`, is written to a file that currently reads as `current`.
    fn applied_to(&self, current: &TrackTags, other: &[Resolved]) -> TrackTags {
        fn text(edit: &FieldEdit<String>, current: &Option<String>) -> Option<String> {
            match edit.normalized() {
                FieldEdit::Keep => current.clone(),
                FieldEdit::Set(value) => Some(value.to_owned()),
                FieldEdit::Clear => None,
            }
        }
        fn value<T: Copy>(edit: &FieldEdit<T>, current: Option<T>) -> Option<T> {
            match edit {
                FieldEdit::Keep => current,
                FieldEdit::Set(value) => Some(*value),
                FieldEdit::Clear => None,
            }
        }
        // MusicBrainz IDs are edited as other tags; the scan reads the first value.
        let id = |item: ItemKey, current: &Option<String>| match other_tags::values_of(other, item)
        {
            None => current.clone(),
            Some(values) => values
                .first()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        };
        TrackTags {
            title: text(&self.title, &current.title),
            artist: text(&self.artist, &current.artist),
            album: text(&self.album, &current.album),
            album_artist: text(&self.album_artist, &current.album_artist),
            track_number: value(&self.track_number, current.track_number),
            disc_number: value(&self.disc_number, current.disc_number),
            year: value(&self.year, current.year),
            genre: text(&self.genre, &current.genre),
            mb_album_id: id(ItemKey::MusicBrainzReleaseId, &current.mb_album_id),
            mb_recording_id: id(ItemKey::MusicBrainzRecordingId, &current.mb_recording_id),
            mb_artist_id: id(ItemKey::MusicBrainzArtistId, &current.mb_artist_id),
            mb_album_artist_id: id(
                ItemKey::MusicBrainzReleaseArtistId,
                &current.mb_album_artist_id,
            ),
        }
    }
}

/// Why the tags of one file were not written. The file is unchanged in every case.
#[derive(Debug, thiserror::Error)]
pub enum TagWriteError {
    #[error("the file is not available")]
    Unavailable,
    #[error("the file is read-only")]
    ReadOnly,
    #[error("the file could not be read: {0}")]
    Unreadable(String),
    #[error("the tags could not be written: {0}")]
    Write(String),
    #[error("the written tags did not read back as expected: {0}")]
    Verify(String),
    #[error("the file changed while its tags were being written")]
    ChangedMeanwhile,
    #[error("the file could not be replaced: {0}")]
    Replace(String),
    /// A key of [`TagEdit::other`] the file's kind of tag can't hold as given: an
    /// ID3v2 key in a FLAC file, a common name the format has no field for, a value
    /// the format takes only in another form (an ID3v2 rating that isn't
    /// `email|stars|count`), or two keys naming the same tag.
    #[error("the file's kind of tag can't hold {0:?} as given")]
    UnsupportedKey(String),
    /// A change set's file no longer holds what the set was made against, so it is
    /// left alone rather than overwritten.
    #[error("the file's tags changed since the change was asked for: {0}")]
    Conflict(String),
}

/// What a change set was made against, which the file must still hold to be
/// written: the values of every key the set may change, and the cover.
#[derive(Debug, Default)]
pub(crate) struct Expected {
    pub values: BTreeMap<String, Vec<String>>,
    /// [`cover_hash`] of the embedded cover, empty for none; None when the set leaves
    /// the cover alone.
    pub cover: Option<String>,
}

/// A hash naming a cover's bytes.
pub(crate) fn cover_hash(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Writes `edit` into the file at `path` through a verified copy.
///
/// `raw` lets [`TagEdit::other`] name the keys of the nine fields, so an undo can put
/// back the values a file had, all of them, instead of the scan's reading of them.
/// With `expected`, a file that doesn't hold those values fails with
/// [`TagWriteError::Conflict`] before anything is copied.
pub(crate) fn write(
    path: &Path,
    edit: &TagEdit,
    raw: bool,
    expected: Option<&Expected>,
) -> Result<(), TagWriteError> {
    // A symlink keeps pointing at the edited file instead of being replaced by it.
    let path = &dunce::canonicalize(path).map_err(|_| TagWriteError::Unavailable)?;
    let metadata = fs::metadata(path).map_err(|_| TagWriteError::Unavailable)?;
    if metadata.permissions().readonly() {
        return Err(TagWriteError::ReadOnly);
    }
    let current = tags::read(path, false).map_err(TagWriteError::Unreadable)?;
    if let Some(expected) = expected {
        check_expected(path, &current.entries, expected)?;
    }
    let other = other_tags::resolve(&edit.other, current.format, raw)
        .map_err(TagWriteError::UnsupportedKey)?;
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(TagWriteError::Unavailable);
    };

    // The copy starts with a dot and doesn't end in a music extension, so neither a
    // scan nor a file manager's default view picks it up. Dropping it deletes it.
    let mut copy = tempfile::Builder::new()
        .prefix(&format!(".{}.", name.to_string_lossy()))
        .suffix(".vcmp-tmp")
        .tempfile_in(dir)
        .map_err(|error| TagWriteError::Write(format!("creating a copy: {error}")))?;
    let write_error = |step: &str| {
        let step = step.to_owned();
        move |error: io::Error| TagWriteError::Write(format!("{step}: {error}"))
    };
    io::copy(
        &mut File::open(path).map_err(write_error("opening the file"))?,
        copy.as_file_mut(),
    )
    .map_err(write_error("copying the file"))?;
    copy.as_file()
        .set_permissions(metadata.permissions())
        .map_err(write_error("copying permissions"))?;
    copy.rewind().map_err(write_error("copying the file"))?;

    let (expected, created) = edit_tag(
        copy.as_file_mut(),
        current.format,
        edit,
        &current.tags,
        &other,
    )
    .map_err(TagWriteError::Write)?;
    copy.as_file()
        .sync_all()
        .map_err(write_error("saving the copy"))?;

    copy.rewind().map_err(write_error("reading the copy"))?;
    let written = snapshot(copy.as_file_mut(), current.format).map_err(TagWriteError::Verify)?;
    if let Some(difference) = expected.difference(&written) {
        return Err(TagWriteError::Verify(difference));
    }
    // Read back the way a scan reads, so the database will hold exactly what was asked.
    let reread = tags::read(copy.path(), false).map_err(TagWriteError::Verify)?;
    let wanted = edit.applied_to(&current.tags, &other);
    if reread.format != current.format {
        return Err(TagWriteError::Verify("the file type changed".into()));
    }
    if reread.duration != current.duration {
        return Err(TagWriteError::Verify(format!(
            "the duration changed from {:?} to {:?}",
            current.duration, reread.duration
        )));
    }
    // Values written raw under the nine fields' keys are checked by the entries below;
    // the scan's reading of them follows from those.
    if !other_tags::names_shown(&other) && reread.tags != wanted {
        return Err(TagWriteError::Verify(format!(
            "expected {wanted:?}, read {:?}",
            reread.tags
        )));
    }
    other_tags::check(&current.entries, &reread.entries, edit, &other, created)
        .map_err(TagWriteError::Verify)?;

    let now = fs::metadata(path).map_err(|_| TagWriteError::Unavailable)?;
    if now.len() != metadata.len() || now.modified().ok() != metadata.modified().ok() {
        return Err(TagWriteError::ChangedMeanwhile);
    }
    replace(copy, path)?;
    Ok(())
}

fn check_expected(
    path: &Path,
    entries: &[TagEntry],
    expected: &Expected,
) -> Result<(), TagWriteError> {
    for (key, values) in &expected.values {
        let found: Vec<&String> = entries
            .iter()
            .filter(|entry| entry.key == *key)
            .map(|entry| &entry.value)
            .collect();
        if !found.iter().copied().eq(values.iter()) {
            return Err(TagWriteError::Conflict(format!(
                "{key} was {values:?}, is {found:?}"
            )));
        }
    }
    if let Some(cover) = &expected.cover {
        let found = tags::embedded_cover(path)
            .map(|data| cover_hash(&data))
            .unwrap_or_default();
        if found != *cover {
            return Err(TagWriteError::Conflict("the cover changed".into()));
        }
    }
    Ok(())
}

/// Moves the verified copy over the original. On Windows an antivirus or indexer can
/// hold the fresh copy for a moment, so the rename is retried briefly there.
fn replace(mut copy: tempfile::NamedTempFile, path: &Path) -> Result<(), TagWriteError> {
    let attempts = if cfg!(windows) { 5 } else { 1 };
    for attempt in 1..=attempts {
        match copy.persist(path) {
            Ok(_) => {
                sync_dir(path);
                return Ok(());
            }
            Err(error) if attempt == attempts => {
                return Err(TagWriteError::Replace(error.error.to_string()));
            }
            Err(error) => {
                copy = error.file;
                std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
            }
        }
    }
    unreachable!("the last attempt returns")
}

/// Makes the rename durable. Best effort: not every file system supports it.
fn sync_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(dir) = File::open(dir)
    {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The parts of the written tag checked after writing, beyond what the scanner reads:
/// every item of the tag, every entry the scan lists (the items lofty has no common
/// name for included) and the pictures.
#[derive(Debug, Default, PartialEq)]
struct Snapshot {
    items: Vec<String>,
    /// Values by entry key, in order.
    entries: BTreeMap<String, Vec<String>>,
    /// Picture type and a hash of the image bytes.
    pictures: Vec<(u8, String)>,
}

impl Snapshot {
    fn id3v2(tag: Option<&Id3v2Tag>) -> Self {
        match tag {
            Some(tag) => Self::new(tag.clone().into(), tags::id3v2_entries(tag), &[]),
            None => Self::default(),
        }
    }

    fn vorbis(
        tag: Option<&VorbisComments>,
        extra_pictures: &[(Picture, PictureInformation)],
    ) -> Self {
        match tag {
            Some(tag) => Self::new(
                tag.clone().into(),
                tags::vorbis_entries(tag),
                extra_pictures,
            ),
            None => Self::default(),
        }
    }

    fn new(
        tag: Tag,
        tag_entries: Vec<TagEntry>,
        extra_pictures: &[(Picture, PictureInformation)],
    ) -> Self {
        let mut items: Vec<String> = tag.items().map(|item| format!("{item:?}")).collect();
        items.sort();
        let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in tag_entries {
            entries.entry(entry.key).or_default().push(entry.value);
        }
        let mut pictures: Vec<(u8, String)> = tag
            .pictures()
            .iter()
            .chain(extra_pictures.iter().map(|(picture, _)| picture))
            .map(|picture| {
                (
                    picture.pic_type().as_u8(),
                    blake3::hash(picture.data()).to_hex().to_string(),
                )
            })
            .collect();
        pictures.sort();
        Self {
            items,
            entries,
            pictures,
        }
    }

    fn difference(&self, written: &Self) -> Option<String> {
        if self.pictures != written.pictures {
            return Some(format!(
                "pictures: expected {:?}, read {:?}",
                self.pictures, written.pictures
            ));
        }
        if self.entries != written.entries {
            return Some(format!(
                "entries: expected {:?}, read {:?}",
                self.entries, written.entries
            ));
        }
        let missing: Vec<&String> = self
            .items
            .iter()
            .filter(|item| !written.items.contains(item))
            .collect();
        let extra: Vec<&String> = written
            .items
            .iter()
            .filter(|item| !self.items.contains(item))
            .collect();
        (!missing.is_empty() || !extra.is_empty())
            .then(|| format!("fields lost: {missing:?}, fields added: {extra:?}"))
    }
}

/// Edits the tag VCMP reads in `file` and saves it. Returns what the tag should read
/// back as, and whether the tag had to be created.
fn edit_tag(
    file: &mut File,
    format: Format,
    edit: &TagEdit,
    current: &TrackTags,
    other: &[Resolved],
) -> Result<(Snapshot, bool), String> {
    let options = ParseOptions::new();
    let read_error = failed("reading the tags");
    match format {
        Format::Mp3 | Format::Wav => {
            let existing = if format == Format::Mp3 {
                MpegFile::read_from(file, options)
                    .map_err(read_error)?
                    .id3v2()
                    .cloned()
            } else {
                WavFile::read_from(file, options)
                    .map_err(read_error)?
                    .id3v2()
                    .cloned()
            };
            let was_id3v23 = existing
                .as_ref()
                .is_some_and(|tag| tag.original_version() != Id3v2Version::V4);
            let created = existing.is_none();
            let mut tag = existing.unwrap_or_else(|| seeded(TagType::Id3v2, current).into());
            edit_id3v2(&mut tag, edit)?;
            other_tags::edit_id3v2(&mut tag, other)?;
            let expected = Snapshot::id3v2(Some(&tag));
            let save = |file: &mut File, use_id3v23: bool| {
                file.rewind().map_err(|error| error.to_string())?;
                tag.save_to(file, WriteOptions::new().use_id3v23(use_id3v23))
                    .map_err(failed("saving the tags"))
            };
            // Keep ID3v2.3, which some players still need, when it loses nothing.
            // Taggers put ID3v2.4 frames (full release dates) into ID3v2.3 tags, and
            // writing ID3v2.3 would cut or drop those, so such a tag becomes ID3v2.4.
            if was_id3v23 {
                save(file, true)?;
                file.rewind().map_err(|error| error.to_string())?;
                if snapshot(file, format)?.difference(&expected).is_none() {
                    return Ok((expected, created));
                }
            }
            save(file, false)?;
            Ok((expected, created))
        }
        Format::OggVorbis => {
            let ogg = VorbisFile::read_from(file, options).map_err(read_error)?;
            let mut tag = ogg.vorbis_comments().clone();
            edit_vorbis(&mut tag, edit);
            other_tags::edit_vorbis(&mut tag, other)?;
            edit_pictures(&mut tag, &edit.cover)?;
            file.rewind().map_err(|error| error.to_string())?;
            tag.save_to(file, WriteOptions::new())
                .map_err(failed("saving the tags"))?;
            Ok((Snapshot::vorbis(Some(&tag), &[]), false))
        }
        Format::Flac => {
            let mut flac = FlacFile::read_from(file, options).map_err(read_error)?;
            let created = flac.vorbis_comments().is_none();
            let mut tag = match flac.vorbis_comments() {
                Some(tag) => tag.clone(),
                None => seeded(TagType::VorbisComments, current).into(),
            };
            edit_vorbis(&mut tag, edit);
            other_tags::edit_vorbis(&mut tag, other)?;
            // Pictures stored inside the comments are written back as picture blocks
            // anyway; keep them all in one list, in the order the scanner sees them.
            let mut pictures = tag.remove_pictures();
            pictures.extend(flac.remove_pictures());
            for (picture, info) in pictures {
                flac.insert_picture(picture, Some(info))
                    .map_err(|error| error.to_string())?;
            }
            edit_pictures(&mut flac, &edit.cover)?;
            flac.set_vorbis_comments(tag.clone());
            // Without this an ID3v2 tag in the FLAC file would be rewritten too.
            flac.remove_id3v2();
            file.rewind().map_err(|error| error.to_string())?;
            flac.save_to(file, WriteOptions::new())
                .map_err(failed("saving the tags"))?;
            Ok((Snapshot::vorbis(Some(&tag), flac.pictures()), created))
        }
    }
}

fn failed<E: std::fmt::Display>(step: &'static str) -> impl Fn(E) -> String {
    move |error| format!("{step}: {error}")
}

/// Reads back what [`edit_tag`] wrote.
fn snapshot(file: &mut File, format: Format) -> Result<Snapshot, String> {
    let options = ParseOptions::new();
    let read_error = failed("reading the copy");
    Ok(match format {
        Format::Mp3 => Snapshot::id3v2(
            MpegFile::read_from(file, options)
                .map_err(read_error)?
                .id3v2(),
        ),
        Format::Wav => Snapshot::id3v2(
            WavFile::read_from(file, options)
                .map_err(read_error)?
                .id3v2(),
        ),
        Format::OggVorbis => Snapshot::vorbis(
            Some(
                VorbisFile::read_from(file, options)
                    .map_err(read_error)?
                    .vorbis_comments(),
            ),
            &[],
        ),
        Format::Flac => {
            let flac = FlacFile::read_from(file, options).map_err(read_error)?;
            Snapshot::vorbis(flac.vorbis_comments(), flac.pictures())
        }
    })
}

/// A new tag holding what the scanner read from the file's other tags (an ID3v1 tag,
/// a RIFF INFO list), so creating the tag VCMP reads loses none of it.
fn seeded(tag_type: TagType, current: &TrackTags) -> Tag {
    let mut tag = Tag::new(tag_type);
    let texts = [
        (ItemKey::TrackTitle, &current.title),
        (ItemKey::TrackArtist, &current.artist),
        (ItemKey::AlbumTitle, &current.album),
        (ItemKey::AlbumArtist, &current.album_artist),
        (ItemKey::Genre, &current.genre),
        (ItemKey::MusicBrainzReleaseId, &current.mb_album_id),
        (ItemKey::MusicBrainzRecordingId, &current.mb_recording_id),
        (ItemKey::MusicBrainzArtistId, &current.mb_artist_id),
        (
            ItemKey::MusicBrainzReleaseArtistId,
            &current.mb_album_artist_id,
        ),
    ];
    for (key, value) in texts {
        if let Some(value) = value {
            tag.insert_text(key, value.clone());
        }
    }
    if let Some(track) = current.track_number {
        tag.set_track(track);
    }
    if let Some(disc) = current.disc_number {
        tag.set_disk(disc);
    }
    if let Some(year) = current.year.and_then(|year| u16::try_from(year).ok()) {
        tag.set_date(year_only(year));
    }
    tag
}

fn year_only(year: u16) -> Timestamp {
    Timestamp {
        year,
        ..Timestamp::default()
    }
}

fn frame_id(id: &'static str) -> FrameId<'static> {
    FrameId::Valid(Cow::Borrowed(id))
}

fn edit_id3v2(tag: &mut Id3v2Tag, edit: &TagEdit) -> Result<(), String> {
    match edit.title.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_title(value.to_owned()),
        FieldEdit::Clear => tag.remove_title(),
    }
    match edit.artist.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_artist(value.to_owned()),
        FieldEdit::Clear => tag.remove_artist(),
    }
    match edit.album.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_album(value.to_owned()),
        FieldEdit::Clear => tag.remove_album(),
    }
    let album_artist = edit.album_artist.normalized();
    if !album_artist.is_keep() {
        let _ = tag.remove(&frame_id("TPE2"));
        // Some taggers store the album artist in a user text frame, which the scanner
        // reads as well.
        tag.retain(|frame| match frame {
            Frame::UserText(text) => !is_album_artist_key(&text.description),
            _ => true,
        });
        if let FieldEdit::Set(value) = album_artist {
            tag.insert(Frame::Text(TextInformationFrame::new(
                frame_id("TPE2"),
                TextEncoding::UTF8,
                value.to_owned(),
            )));
        }
    }
    match edit.track_number {
        FieldEdit::Keep => {}
        // The track total, if any, is kept.
        FieldEdit::Set(value) => tag.set_track(value),
        FieldEdit::Clear => tag.remove_track(),
    }
    match edit.disc_number {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_disk(value),
        FieldEdit::Clear => tag.remove_disk(),
    }
    if !edit.year.is_keep() {
        for id in ["TDRC", "TYER", "TDAT", "TIME"] {
            let _ = tag.remove(&frame_id(id));
        }
        if let FieldEdit::Set(year) = edit.year {
            // A timestamp frame, not a text frame: lofty drops a text TDRC when it
            // writes ID3v2.3.
            tag.insert(Frame::Timestamp(TimestampFrame::new(
                frame_id("TDRC"),
                TextEncoding::UTF8,
                year_only(year as u16),
            )));
        }
    }
    match edit.genre.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_genre(value.to_owned()),
        FieldEdit::Clear => tag.remove_genre(),
    }

    if !edit.cover.is_keep() {
        let types: Vec<PictureType> = tag
            .iter()
            .filter_map(|frame| match frame {
                Frame::Picture(frame) => Some(frame.picture.pic_type()),
                _ => None,
            })
            .collect();
        let remove = cover_indexes(&types);
        let mut index = 0;
        tag.retain(|frame| {
            if !matches!(frame, Frame::Picture(_)) {
                return true;
            }
            index += 1;
            !remove.contains(&(index - 1))
        });
        if let FieldEdit::Set(data) = &edit.cover {
            tag.insert_picture(front_cover(data)?);
        }
    }
    Ok(())
}

fn edit_vorbis(tag: &mut VorbisComments, edit: &TagEdit) {
    let original = other_tags::vorbis_fields(tag);
    edit_vorbis_fields(tag, edit);
    other_tags::keep_vorbis_order(tag, &original);
}

fn edit_vorbis_fields(tag: &mut VorbisComments, edit: &TagEdit) {
    match edit.title.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_title(value.to_owned()),
        FieldEdit::Clear => tag.remove_title(),
    }
    match edit.artist.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_artist(value.to_owned()),
        FieldEdit::Clear => tag.remove_artist(),
    }
    match edit.album.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_album(value.to_owned()),
        FieldEdit::Clear => tag.remove_album(),
    }
    let album_artist = edit.album_artist.normalized();
    if !album_artist.is_keep() {
        let _ = tag.remove("ALBUMARTIST");
        let _ = tag.remove("ALBUM ARTIST");
        if let FieldEdit::Set(value) = album_artist {
            tag.insert("ALBUMARTIST".into(), value.to_owned());
        }
    }
    match edit.track_number {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_track(value),
        FieldEdit::Clear => {
            tag.remove_track();
            tag.remove_track_total();
        }
    }
    match edit.disc_number {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_disk(value),
        FieldEdit::Clear => {
            tag.remove_disk();
            tag.remove_disk_total();
        }
    }
    match edit.year {
        FieldEdit::Keep => {}
        FieldEdit::Set(year) => tag.set_date(year_only(year as u16)),
        FieldEdit::Clear => tag.remove_date(),
    }
    match edit.genre.normalized() {
        FieldEdit::Keep => {}
        FieldEdit::Set(value) => tag.set_genre(value.to_owned()),
        FieldEdit::Clear => tag.remove_genre(),
    }
}

pub(crate) fn is_album_artist_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("ALBUMARTIST") || key.eq_ignore_ascii_case("ALBUM ARTIST")
}

fn edit_pictures(
    storage: &mut impl OggPictureStorage,
    cover: &FieldEdit<Vec<u8>>,
) -> Result<(), String> {
    if cover.is_keep() {
        return Ok(());
    }
    let types: Vec<PictureType> = storage
        .pictures()
        .iter()
        .map(|(picture, _)| picture.pic_type())
        .collect();
    for index in cover_indexes(&types).into_iter().rev() {
        storage.remove_picture(index);
    }
    if let FieldEdit::Set(data) = cover {
        storage
            .insert_picture(front_cover(data)?, None)
            .map_err(|error| format!("the cover: {error}"))?;
    }
    Ok(())
}

/// The pictures VCMP treats as the cover: every front cover, or when there is none, a
/// leading picture without a type (many taggers save the cover that way). Setting
/// or removing the cover replaces these and keeps every other picture.
fn cover_indexes(types: &[PictureType]) -> Vec<usize> {
    let fronts: Vec<usize> = types
        .iter()
        .enumerate()
        .filter(|(_, kind)| **kind == PictureType::CoverFront)
        .map(|(index, _)| index)
        .collect();
    if fronts.is_empty() && types.first() == Some(&PictureType::Other) {
        return vec![0];
    }
    fronts
}

fn front_cover(data: &[u8]) -> Result<Picture, String> {
    let mime_type = cover_mime_type(data).ok_or_else(|| InvalidTagEdit::CoverFormat.to_string())?;
    Ok(Picture::unchecked(data.to_vec())
        .pic_type(PictureType::CoverFront)
        .mime_type(mime_type)
        .build())
}

/// JPEG and PNG are the cover formats every player reads from tags.
fn cover_mime_type(data: &[u8]) -> Option<MimeType> {
    match image::guess_format(data).ok()? {
        image::ImageFormat::Jpeg => Some(MimeType::Jpeg),
        image::ImageFormat::Png => Some(MimeType::Png),
        _ => None,
    }
}
