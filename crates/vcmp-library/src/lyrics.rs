//! Lyrics for the now playing view: read from a track's file and from the `.lrc`
//! file next to it when the track starts, plus what LRCLIB answered, which is kept
//! in the database. [`choose_lyrics`] picks what is shown, in a fixed order: the
//! `.lrc` file, embedded synced lyrics, LRCLIB's synced lyrics, then plain text in
//! the same order of sources. Nothing here is written to a music file.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, Id3v2Tag, SynchronizedTextFrame, TimestampFormat};
use lofty::iff::wav::WavFile;
use lofty::mpeg::MpegFile;
use lofty::ogg::VorbisFile;
use lofty::ogg::tag::VorbisComments;
use lofty::probe::Probe;
use rusqlite::{OptionalExtension, params};

use crate::{Library, Result, TrackId};

/// One line of synced lyrics and when it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricLine {
    pub start: Duration,
    /// Empty for a gap between verses.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsText {
    /// Lines with their start times, in time order.
    Synced(Vec<LyricLine>),
    /// Text without times, its lines as written.
    Plain(String),
}

/// Where shown lyrics came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LyricsSource {
    /// The `.lrc` file next to the track.
    LrcFile,
    /// The track's own tags.
    Embedded,
    /// LRCLIB, through the database's cache.
    Online,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lyrics {
    pub source: LyricsSource,
    pub text: LyricsText,
}

/// The lyrics a track has on disk: in the `.lrc` file of its name, and in its tags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalLyrics {
    pub lrc_file: Option<LyricsText>,
    /// From an ID3v2 SYLT frame, or tag text in LRC form.
    pub embedded_synced: Option<Vec<LyricLine>>,
    pub embedded_plain: Option<String>,
}

impl LocalLyrics {
    /// Synced lyrics are on disk, so LRCLIB need not be asked.
    pub fn has_synced(&self) -> bool {
        matches!(self.lrc_file, Some(LyricsText::Synced(_))) || self.embedded_synced.is_some()
    }
}

/// What LRCLIB has for a track, as it sent it: LRC text and plain text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnlineLyrics {
    pub synced: Option<String>,
    pub plain: Option<String>,
}

/// The lyrics to show: synced before plain, and within each the `.lrc` file, then
/// the tags, then LRCLIB. `None` when there are none.
pub fn choose_lyrics(local: &LocalLyrics, online: Option<&OnlineLyrics>) -> Option<Lyrics> {
    let lrc_synced = match &local.lrc_file {
        Some(LyricsText::Synced(lines)) => Some(lines.clone()),
        _ => None,
    };
    let lrc_plain = match &local.lrc_file {
        Some(LyricsText::Plain(text)) => Some(text.clone()),
        _ => None,
    };
    let online_synced = online
        .and_then(|online| online.synced.as_deref())
        .and_then(parse_lrc);
    let online_plain = online
        .and_then(|online| online.plain.as_deref())
        .and_then(plain_text);
    let synced = [
        (LyricsSource::LrcFile, lrc_synced),
        (LyricsSource::Embedded, local.embedded_synced.clone()),
        (LyricsSource::Online, online_synced),
    ]
    .into_iter()
    .find_map(|(source, lines)| {
        lines.map(|lines| Lyrics {
            source,
            text: LyricsText::Synced(lines),
        })
    });
    synced.or_else(|| {
        [
            (LyricsSource::LrcFile, lrc_plain),
            (LyricsSource::Embedded, local.embedded_plain.clone()),
            (LyricsSource::Online, online_plain),
        ]
        .into_iter()
        .find_map(|(source, text)| {
            text.map(|text| Lyrics {
                source,
                text: LyricsText::Plain(text),
            })
        })
    })
}

/// `.lrc` files larger than this are not read; lyrics run to a few kilobytes.
const LRC_FILE_LIMIT: u64 = 1024 * 1024;

/// Reads the lyrics of the track at `path`: the `.lrc` file of the same name, and
/// the lyrics in its tags. Anything unreadable counts as absent.
pub fn read_local_lyrics(path: &Path) -> LocalLyrics {
    let mut local = LocalLyrics {
        lrc_file: read_lrc_file(path),
        ..LocalLyrics::default()
    };
    let mut texts = Vec::new();
    let mut synced = None;
    match read_tag_lyrics(path) {
        Some(TagLyrics::Id3v2(tag)) => {
            synced = sylt_lines(&tag);
            texts = uslt_texts(&tag);
        }
        Some(TagLyrics::Vorbis(texts_found)) => texts = texts_found,
        None => {}
    }
    for text in &texts {
        if synced.is_none() {
            synced = parse_lrc(text);
        }
    }
    local.embedded_synced = synced;
    local.embedded_plain = texts
        .iter()
        .filter(|text| parse_lrc(text).is_none())
        .find_map(|text| plain_text(text));
    local
}

/// The `.lrc` file next to `path`: its extension replaced by `lrc` or `LRC`.
fn lrc_path(path: &Path) -> Option<PathBuf> {
    ["lrc", "LRC"]
        .into_iter()
        .map(|extension| path.with_extension(extension))
        .find(|candidate| candidate.is_file())
}

fn read_lrc_file(path: &Path) -> Option<LyricsText> {
    let lrc = lrc_path(path)?;
    let size = std::fs::metadata(&lrc).ok()?.len();
    if size > LRC_FILE_LIMIT {
        log::info!("{} is too large to be lyrics", lrc.display());
        return None;
    }
    let text = decode_text(&std::fs::read(&lrc).ok()?);
    match parse_lrc(&text) {
        Some(lines) => Some(LyricsText::Synced(lines)),
        None => plain_text(&text).map(LyricsText::Plain),
    }
}

/// Text from a file of unknown encoding: UTF-8 with or without a BOM, UTF-16 with a
/// BOM, and anything else as Latin-1.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| from(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&byte| char::from(byte)).collect(),
    }
}

/// Plain lyrics as written, without the blank lines around them; `None` when there
/// is no text.
fn plain_text(text: &str) -> Option<String> {
    let text = text.trim_matches(|c: char| c == '\n' || c == '\r');
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    (!text.trim().is_empty()).then_some(text)
}

enum TagLyrics {
    Id3v2(Id3v2Tag),
    /// The lyrics fields' values.
    Vorbis(Vec<String>),
}

/// The tag VCMP reads lyrics from: ID3v2 in MP3 and WAV files, Vorbis comments in
/// FLAC and Ogg Vorbis files.
fn read_tag_lyrics(path: &Path) -> Option<TagLyrics> {
    let options = ParseOptions::new().read_cover_art(false);
    let probe = Probe::open(path)
        .ok()?
        .options(options)
        .guess_file_type()
        .ok()?;
    let file_type = probe.file_type()?;
    let mut reader = probe.into_inner();
    let vorbis = |tag: &VorbisComments| {
        let texts = ["LYRICS", "UNSYNCEDLYRICS"]
            .into_iter()
            .flat_map(|key| tag.get_all(key))
            .map(str::to_owned)
            .collect();
        TagLyrics::Vorbis(texts)
    };
    match file_type {
        FileType::Mpeg => MpegFile::read_from(&mut reader, options)
            .ok()?
            .id3v2()
            .cloned()
            .map(TagLyrics::Id3v2),
        FileType::Wav => WavFile::read_from(&mut reader, options)
            .ok()?
            .id3v2()
            .cloned()
            .map(TagLyrics::Id3v2),
        FileType::Flac => FlacFile::read_from(&mut reader, options)
            .ok()?
            .vorbis_comments()
            .map(vorbis),
        FileType::Vorbis => Some(vorbis(
            VorbisFile::read_from(&mut reader, options)
                .ok()?
                .vorbis_comments(),
        )),
        _ => None,
    }
}

/// The texts of the USLT frames, those without a description first.
fn uslt_texts(tag: &Id3v2Tag) -> Vec<String> {
    let mut texts: Vec<(bool, String)> = tag
        .into_iter()
        .filter_map(|frame| match frame {
            Frame::UnsynchronizedText(frame) => {
                Some((!frame.description.is_empty(), frame.content.to_string()))
            }
            _ => None,
        })
        .collect();
    texts.sort_by_key(|(described, _)| *described);
    texts.into_iter().map(|(_, text)| text).collect()
}

/// The lines of the first SYLT frame timed in milliseconds that holds any. lofty
/// keeps SYLT as binary data and parses it when asked.
fn sylt_lines(tag: &Id3v2Tag) -> Option<Vec<LyricLine>> {
    tag.into_iter()
        .filter_map(|frame| match frame {
            Frame::Binary(binary) if frame.id_str() == "SYLT" => {
                SynchronizedTextFrame::parse(&binary.data, frame.flags()).ok()
            }
            _ => None,
        })
        .filter(|sylt| sylt.timestamp_format == TimestampFormat::MS)
        .find_map(|sylt| sylt_to_lines(&sylt.content))
}

/// SYLT entries as lines. Where entries mark new lines with a leading line break,
/// those without one continue the line before (syllables); otherwise every entry is
/// a line.
fn sylt_to_lines(content: &[(u32, String)]) -> Option<Vec<LyricLine>> {
    let breaks = content
        .iter()
        .any(|(_, text)| text.starts_with(['\n', '\r']));
    let mut lines: Vec<LyricLine> = Vec::new();
    for (ms, text) in content {
        let starts_line = !breaks || text.starts_with(['\n', '\r']) || lines.is_empty();
        let text = text.trim_matches(['\n', '\r']);
        if starts_line {
            lines.push(LyricLine {
                start: Duration::from_millis(u64::from(*ms)),
                text: text.trim().to_owned(),
            });
        } else if let Some(line) = lines.last_mut() {
            line.text.push_str(text);
        }
    }
    for line in &mut lines {
        line.text = line.text.trim().to_owned();
    }
    lines.sort_by_key(|line| line.start);
    lines
        .iter()
        .any(|line| !line.text.is_empty())
        .then_some(lines)
}

/// Parses LRC text: `[mm:ss.xx]` stamps (several on a line repeat it at each),
/// `[offset:+/-ms]`, other `[tag:value]` lines ignored, word stamps `<mm:ss.xx>`
/// removed. Lines without a stamp are dropped. `None` when no line has a stamp.
pub fn parse_lrc(text: &str) -> Option<Vec<LyricLine>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut offset_ms: i64 = 0;
    let mut stamped: Vec<(i64, String)> = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start();
        let mut stamps = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else {
                break;
            };
            let tag = &inner[..end];
            if let Some(ms) = parse_stamp(tag) {
                stamps.push(ms);
            } else if let Some(value) = tag
                .split_once(':')
                .filter(|(key, _)| key.trim().eq_ignore_ascii_case("offset"))
                .and_then(|(_, value)| value.trim().trim_start_matches('+').parse::<i64>().ok())
            {
                offset_ms = value;
            }
            rest = inner[end + 1..].trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        let words = without_word_stamps(rest);
        for ms in stamps {
            stamped.push((ms, words.clone()));
        }
    }
    if stamped.is_empty() {
        return None;
    }
    // A positive offset shows the lyrics earlier.
    let mut lines: Vec<LyricLine> = stamped
        .into_iter()
        .map(|(ms, text)| LyricLine {
            start: Duration::from_millis(ms.saturating_sub(offset_ms).max(0) as u64),
            text,
        })
        .collect();
    lines.sort_by_key(|line| line.start);
    Some(lines)
}

/// `mm:ss`, `mm:ss.x` to `mm:ss.xxx` (or `mm:ss:xx`) in milliseconds.
fn parse_stamp(tag: &str) -> Option<i64> {
    let (minutes, rest) = tag.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = match rest.split_once(['.', ':']) {
        Some((seconds, fraction)) => (seconds, Some(fraction)),
        None => (rest, None),
    };
    if seconds.is_empty() || seconds.len() > 2 || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let fraction_ms = match fraction {
        None => 0,
        Some(digits)
            if !digits.is_empty()
                && digits.len() <= 3
                && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            let value: i64 = digits.parse().ok()?;
            value * 10_i64.pow(3 - digits.len() as u32)
        }
        Some(_) => return None,
    };
    let minutes: i64 = minutes.parse().ok()?;
    let seconds: i64 = seconds.parse().ok()?;
    minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(fraction_ms)
}

/// The line without its `<mm:ss.xx>` word stamps, trimmed.
fn without_word_stamps(line: &str) -> String {
    let mut text = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        text.push_str(&rest[..start]);
        let inner = &rest[start + 1..start + end];
        if parse_stamp(inner).is_none() {
            text.push_str(&rest[start..=start + end]);
        }
        rest = &rest[start + end + 1..];
    }
    text.push_str(rest);
    text.trim().to_owned()
}

/// A track whose lyrics LRCLIB may be asked for, by where it comes from and its id
/// there, so other sources than the library can share the cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LyricsKey {
    pub source: String,
    pub id: String,
}

impl LyricsKey {
    /// A track of the library.
    pub fn local(track: TrackId) -> Self {
        Self {
            source: "local".into(),
            id: track.0.to_string(),
        }
    }
}

/// What LRCLIB is asked: the track as its tags name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Whole seconds, as LRCLIB takes it.
    pub duration_secs: u32,
}

/// What LRCLIB answered for a track, to which question, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsLookup {
    pub query: LyricsQuery,
    /// `None` when LRCLIB had nothing.
    pub found: Option<OnlineLyrics>,
    pub looked_up: SystemTime,
}

/// A track LRCLIB had nothing for is asked again after this long.
pub const LYRICS_RETRY_NOT_FOUND: Duration = Duration::from_secs(30 * 24 * 60 * 60);

impl LyricsLookup {
    /// The stored answer counts for `query`: it was asked the same question.
    pub fn answers(&self, query: &LyricsQuery) -> bool {
        self.query == *query
    }
}

/// Whether LRCLIB should be asked `query` now, given the stored answer: never asked,
/// asked something else (the tags changed since), or nothing found long enough ago.
pub fn lyrics_lookup_due(stored: Option<&LyricsLookup>, query: &LyricsQuery) -> bool {
    match stored {
        None => true,
        Some(lookup) if !lookup.answers(query) => true,
        Some(LyricsLookup {
            found: None,
            looked_up,
            ..
        }) => looked_up
            .elapsed()
            .is_ok_and(|age| age >= LYRICS_RETRY_NOT_FOUND),
        Some(_) => false,
    }
}

impl Library {
    /// The stored LRCLIB answer for `key`, if it was looked up.
    pub fn lyrics_lookup(&self, key: &LyricsKey) -> Result<Option<LyricsLookup>> {
        let row = self
            .conn
            .query_row(
                "SELECT title, artist, album, duration_s, synced, plain, looked_up_ms
                   FROM lyrics_lookups WHERE source = ?1 AND track = ?2",
                [&key.source, &key.id],
                |row| {
                    Ok((
                        LyricsQuery {
                            title: row.get(0)?,
                            artist: row.get(1)?,
                            album: row.get(2)?,
                            duration_secs: row.get(3)?,
                        },
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(query, synced, plain, at)| LyricsLookup {
            query,
            found: (synced.is_some() || plain.is_some()).then_some(OnlineLyrics { synced, plain }),
            looked_up: UNIX_EPOCH + Duration::from_millis(at.max(0) as u64),
        }))
    }

    /// Stores LRCLIB's answer to `query` for `key` at `at`; `found` is `None` when
    /// it had nothing. Replaces an earlier answer.
    pub fn set_lyrics_lookup(
        &self,
        key: &LyricsKey,
        query: &LyricsQuery,
        found: Option<&OnlineLyrics>,
        at: SystemTime,
    ) -> Result<()> {
        let at = at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        self.conn.execute(
            "INSERT INTO lyrics_lookups
                 (source, track, title, artist, album, duration_s, synced, plain, looked_up_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (source, track) DO UPDATE SET
                 title = excluded.title,
                 artist = excluded.artist,
                 album = excluded.album,
                 duration_s = excluded.duration_s,
                 synced = excluded.synced,
                 plain = excluded.plain,
                 looked_up_ms = excluded.looked_up_ms",
            params![
                key.source,
                key.id,
                query.title,
                query.artist,
                query.album,
                query.duration_secs,
                found.and_then(|found| found.synced.as_deref()),
                found.and_then(|found| found.plain.as_deref()),
                at,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_stamp_too_large_to_hold_is_left_out() {
        let lines = super::parse_lrc(
            "[offset:-9223372036854775807]
[999999999999999999:00]Never
[00:01]One",
        )
        .unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "One");
    }

    use super::*;

    fn line(ms: u64, text: &str) -> LyricLine {
        LyricLine {
            start: Duration::from_millis(ms),
            text: text.into(),
        }
    }

    #[test]
    fn stamps_offsets_and_word_stamps() {
        let text = "[ar:Band]\n[ti:Song]\n[offset:+500]\n\
                    [00:12.00]First\n\
                    [00:17.20][01:02.5]Chorus <00:17.40>with <00:17.80>words\n\
                    no stamp here\n\
                    [00:20.123]\n\
                    [02:00:50]Colon fraction";
        assert_eq!(
            parse_lrc(text).unwrap(),
            [
                line(11_500, "First"),
                line(16_700, "Chorus with words"),
                line(19_623, ""),
                line(62_000, "Chorus with words"),
                line(120_000, "Colon fraction"),
            ]
        );
    }

    #[test]
    fn a_negative_offset_shows_lines_later_and_none_go_below_zero() {
        assert_eq!(
            parse_lrc("[offset:-250]\n[00:01.00]a").unwrap(),
            [line(1250, "a")]
        );
        assert_eq!(
            parse_lrc("[offset:2000]\n[00:01.00]a").unwrap(),
            [line(0, "a")]
        );
    }

    #[test]
    fn text_without_stamps_is_not_lrc() {
        assert_eq!(parse_lrc("Just words\n[chorus]\n"), None);
        assert_eq!(parse_lrc("[ar:Band]\n[ti:Song]"), None);
    }

    #[test]
    fn sylt_entries_become_lines_or_join_into_them() {
        let entries = [(1000, "One".to_string()), (2000, "Two".to_string())];
        assert_eq!(
            sylt_to_lines(&entries).unwrap(),
            [line(1000, "One"), line(2000, "Two")]
        );
        let syllables = [
            (1000, "\nHel".to_string()),
            (1200, "lo".to_string()),
            (2000, "\nWorld".to_string()),
        ];
        assert_eq!(
            sylt_to_lines(&syllables).unwrap(),
            [line(1000, "Hello"), line(2000, "World")]
        );
    }

    #[test]
    fn encodings_of_lrc_files() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFa\xC3\xA9"), "a\u{e9}");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'a', 0, 0xE9, 0]), "a\u{e9}");
        assert_eq!(decode_text(&[0xFE, 0xFF, 0, b'a', 0, 0xE9]), "a\u{e9}");
        assert_eq!(decode_text(b"caf\xE9"), "caf\u{e9}");
    }

    #[test]
    fn synced_first_then_local_first() {
        let synced = |text: &str| Some(vec![line(0, text)]);
        let online = OnlineLyrics {
            synced: Some("[00:00.00]online".into()),
            plain: Some("online plain".into()),
        };
        let pick = |local: &LocalLyrics, online: Option<&OnlineLyrics>| {
            choose_lyrics(local, online).map(|lyrics| (lyrics.source, lyrics.text))
        };
        let full = LocalLyrics {
            lrc_file: Some(LyricsText::Synced(vec![line(0, "lrc")])),
            embedded_synced: synced("tag"),
            embedded_plain: Some("tag plain".into()),
        };
        assert_eq!(
            pick(&full, Some(&online)),
            Some((
                LyricsSource::LrcFile,
                LyricsText::Synced(vec![line(0, "lrc")])
            ))
        );
        let tags = LocalLyrics {
            lrc_file: None,
            ..full.clone()
        };
        assert_eq!(
            pick(&tags, Some(&online)).unwrap().0,
            LyricsSource::Embedded
        );
        let plain_only = LocalLyrics {
            embedded_synced: None,
            ..tags.clone()
        };
        assert_eq!(
            pick(&plain_only, Some(&online)),
            Some((
                LyricsSource::Online,
                LyricsText::Synced(vec![line(0, "online")])
            )),
            "online synced beats local plain"
        );
        assert_eq!(
            pick(&plain_only, None),
            Some((
                LyricsSource::Embedded,
                LyricsText::Plain("tag plain".into())
            ))
        );
        let lrc_plain = LocalLyrics {
            lrc_file: Some(LyricsText::Plain("lrc plain".into())),
            ..plain_only.clone()
        };
        assert_eq!(
            pick(&lrc_plain, None),
            Some((LyricsSource::LrcFile, LyricsText::Plain("lrc plain".into())))
        );
        let online_plain = OnlineLyrics {
            synced: None,
            plain: Some("online plain".into()),
        };
        assert_eq!(
            pick(&LocalLyrics::default(), Some(&online_plain)),
            Some((
                LyricsSource::Online,
                LyricsText::Plain("online plain".into())
            ))
        );
        assert_eq!(pick(&LocalLyrics::default(), None), None);
    }

    #[test]
    fn a_lookup_is_due_when_never_asked_asked_otherwise_or_missed_long_ago() {
        let query = LyricsQuery {
            title: "Song".into(),
            artist: "Band".into(),
            album: Some("Record".into()),
            duration_secs: 200,
        };
        let found = LyricsLookup {
            query: query.clone(),
            found: Some(OnlineLyrics::default()),
            looked_up: SystemTime::now(),
        };
        assert!(lyrics_lookup_due(None, &query));
        assert!(!lyrics_lookup_due(Some(&found), &query));
        let renamed = LyricsQuery {
            title: "Song (Live)".into(),
            ..query.clone()
        };
        assert!(lyrics_lookup_due(Some(&found), &renamed));
        let longer = LyricsQuery {
            duration_secs: 201,
            ..query.clone()
        };
        assert!(lyrics_lookup_due(Some(&found), &longer));
        let missed = LyricsLookup {
            found: None,
            ..found.clone()
        };
        assert!(!lyrics_lookup_due(Some(&missed), &query));
        let missed_long_ago = LyricsLookup {
            looked_up: SystemTime::now() - LYRICS_RETRY_NOT_FOUND - Duration::from_secs(60),
            ..missed
        };
        assert!(lyrics_lookup_due(Some(&missed_long_ago), &query));
    }
}
