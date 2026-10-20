//! Checks tag writing against real files without touching them: copies a sample of
//! each format into `target/edit-check/`, then edits every copy twice (set every
//! field, a cover and other tags, then put them back and remove the cover) and prints
//! what failed and why. The other tags are a composer, a comment, a key of the
//! format's own and the first other tag the file already has; after the second edit
//! every tag outside the nine fields must read as it did before. Then the first edit
//! goes through a change set, is accepted and undone, and every tag and the cover
//! must be as before. The music directory is only read.
//!
//!     cargo run --release -p vcmp-library --example edit_check -- <music dir> [files per format]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use lofty::file::TaggedFileExt;
use lofty::tag::TagType;
use vcmp_library::{Error, FieldEdit, FileResult, Format, Library, TagEdit, TagEntry, Track};
use walkdir::WalkDir;

/// Keys the nine fields write, and the native keys their edits also remove (album
/// artists in user text frames, ID3v2.3 date parts). Putting the fields back doesn't
/// restore these exactly, so the final comparison leaves them out.
const FIELD_KEYS: [&str; 15] = [
    "TrackTitle",
    "TrackArtist",
    "AlbumTitle",
    "AlbumArtist",
    "TrackNumber",
    "DiscNumber",
    "RecordingDate",
    "Year",
    "Genre",
    "id3:TXXX:ALBUMARTIST",
    "id3:TXXX:ALBUM ARTIST",
    "id3:TDRC",
    "id3:TYER",
    "id3:TDAT",
    "id3:TIME",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().ok_or("usage: edit_check <music dir> [n]")?);
    let per_format: usize = args.next().map(|n| n.parse()).transpose()?.unwrap_or(40);

    let work = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/edit-check");
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    let music = work.join("music");
    std::fs::create_dir_all(&music)?;

    // An even spread over each format, so many albums and taggers show up.
    let mut by_format: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in WalkDir::new(&source).sort_by_file_name() {
        let entry = entry?;
        let extension = entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if let Some(extension) = extension
            && ["mp3", "flac", "ogg", "wav"].contains(&extension.as_str())
            && entry.file_type().is_file()
        {
            by_format
                .entry(extension)
                .or_default()
                .push(entry.into_path());
        }
    }
    for (format, files) in &by_format {
        let step = (files.len() / per_format).max(1);
        let picked: Vec<&PathBuf> = files.iter().step_by(step).take(per_format).collect();
        println!(
            "{format}: copying {} of {} files",
            picked.len(),
            files.len()
        );
        for (index, file) in picked.into_iter().enumerate() {
            let name = file.file_name().unwrap().to_string_lossy();
            std::fs::copy(file, music.join(format!("{format}-{index:03}-{name}")))?;
        }
    }

    let mut library = Library::open(&work.join("library.sqlite3"), &work.join("cache"))?;
    library.add_source(&music, true)?;
    let report = library.scan(&AtomicBool::new(false), |_| {})?;
    for skipped in &report.skipped {
        println!(
            "not scanned: {}: {}",
            skipped.path.display(),
            skipped.reason
        );
    }

    let cover = cover();
    let tracks = library.tracks()?;
    let mut failures = 0;
    let mut not_compared = 0;
    for track in &tracks {
        let entries_before = library.track_tags(track.id)?;
        let before = others(&entries_before);
        // Without the kind of tag VCMP writes (a WAV with only RIFF INFO), the first
        // edit creates one, and its tags are listed from it from then on, some under
        // other names (RIFF INFO's encoder is ID3v2's encoder settings).
        let has_own_tag = lofty::read_from_path(&track.path)
            .map(|file| match track.format {
                Some(Format::Mp3 | Format::Wav) => file.tag(TagType::Id3v2).is_some(),
                Some(Format::Flac) => file.tag(TagType::VorbisComments).is_some(),
                _ => true,
            })
            .unwrap_or(true);
        let native = match track.format {
            Some(Format::Mp3 | Format::Wav) => "id3:TXXX:VCMP EDIT CHECK",
            _ => "vorbis:VCMPEDITCHECK",
        };
        // The first tag the file has beyond the nine fields, rewritten with a mark.
        // Values with control characters (a stray encoding byte in a URL frame) are
        // left out: VCMP refuses to write those, so they couldn't be put back.
        let existing = before
            .iter()
            .find(|(key, values)| {
                !["Composer", "Comment"].contains(&key.as_str())
                    && values.iter().all(|value| {
                        !value
                            .chars()
                            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                    })
            })
            .map(|(key, _)| key.clone());
        let mut set_other = BTreeMap::from([
            ("Composer".to_owned(), set(&["Edit check composer"])),
            ("Comment".to_owned(), set(&["Edit check\nsecond line"])),
            (native.to_owned(), set(&["Edit check"])),
        ]);
        let mut restore_other = BTreeMap::from([
            (
                "Composer".to_owned(),
                restore_values(before.get("Composer")),
            ),
            ("Comment".to_owned(), restore_values(before.get("Comment"))),
            (native.to_owned(), FieldEdit::Clear),
        ]);
        if let Some(key) = existing {
            // Numbers stay numbers: totals and movement numbers take nothing else.
            // Values in lofty's `a|b|c` form (a rating) are written back unchanged,
            // since ID3v2 keeps nothing else.
            let marked: Vec<String> = before[&key]
                .iter()
                .map(|value| match value.parse::<u64>() {
                    Ok(number) => (number + 1).to_string(),
                    Err(_) if value.contains('|') => value.clone(),
                    Err(_) => format!("{value} ✓"),
                })
                .collect();
            set_other.insert(key.clone(), FieldEdit::Set(marked));
            restore_other.insert(key.clone(), restore_values(before.get(&key)));
        }
        let set_all = TagEdit {
            title: FieldEdit::Set(format!("{} ✓ Příliš žluťoučký", track.title)),
            artist: FieldEdit::Set("Edit check artist".into()),
            album_artist: FieldEdit::Set("Edit check album artist".into()),
            track_number: FieldEdit::Set(track.track_number.unwrap_or(0) + 1),
            year: FieldEdit::Set(2001),
            genre: FieldEdit::Set("Edit check".into()),
            cover: FieldEdit::Set(cover.clone()),
            other: set_other,
            ..TagEdit::default()
        };
        let restore = TagEdit {
            title: FieldEdit::Set(track.title.clone()),
            artist: restore(&track.artist),
            album_artist: restore(&track.album_artist),
            track_number: track.track_number.map_or(FieldEdit::Clear, FieldEdit::Set),
            year: track.year.map_or(FieldEdit::Clear, FieldEdit::Set),
            genre: restore(&track.genre),
            cover: FieldEdit::Clear,
            other: restore_other,
            ..TagEdit::default()
        };
        let mut restored = true;
        for (step, edit) in [("set", &set_all), ("restore", &restore)] {
            let report =
                match library.write_tags(&[(track.id, edit)], &AtomicBool::new(false), |_, _| {}) {
                    Ok(report) => report,
                    // A tag the file has that VCMP can't write (a rating, say).
                    Err(Error::InvalidTagEdit(invalid)) => {
                        failures += 1;
                        restored = false;
                        println!("{step} refused: {}: {invalid}", name(track));
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
            for failed in &report.failed {
                failures += 1;
                restored = false;
                println!("{step} failed: {}: {}", name(track), failed.error);
            }
            if !report.written.is_empty() {
                check(&library, track, step)?;
            }
        }
        if !has_own_tag {
            not_compared += 1;
        } else if restored {
            let after = others(&library.track_tags(track.id)?);
            if after != before {
                failures += 1;
                println!(
                    "restore: {}: other tags were {before:?}, are {after:?}",
                    name(track)
                );
            }
        }
        if restored && let Err(problem) = round_trip(&mut library, track, &set_all)? {
            failures += 1;
            println!("change set: {}: {problem}", name(track));
        }
    }
    println!("{} files, {failures} failed edits", tracks.len());
    if not_compared > 0 {
        println!(
            "{not_compared} files had no tag of the kind VCMP writes; their other tags were not compared after the edits"
        );
    }
    Ok(())
}

/// Writes `edit` as a change set and undoes it the same way: afterwards every tag of
/// the file, and its cover, must be as before. By now the file has the kind of tag
/// VCMP writes, so the comparison covers every file.
fn round_trip(
    library: &mut Library,
    track: &Track,
    edit: &TagEdit,
) -> Result<Result<(), String>, Box<dyn std::error::Error>> {
    let sorted = |mut entries: Vec<TagEntry>| {
        entries.sort_by(|a, b| a.key.cmp(&b.key).then(a.value.cmp(&b.value)));
        entries
    };
    let before = sorted(library.track_tags(track.id)?);
    let cover_before = library.embedded_cover(track.id)?;
    let set = match library.submit_tag_changes("edit check", &[(track.id, edit.clone())]) {
        Ok(set) => set,
        Err(error) => return Ok(Err(format!("not recorded: {error}"))),
    };
    let accepted = library.accept_change_set(set, |_, _| {})?;
    if accepted.files[0].result != Some(FileResult::Written) {
        return Ok(Err(format!("set: {:?}", accepted.files[0].result)));
    }
    let undo = match library.submit_undo(set, "edit check") {
        Ok(undo) => undo,
        Err(error) => return Ok(Err(format!("undo not recorded: {error}"))),
    };
    let undone = library.accept_change_set(undo, |_, _| {})?;
    if undone.files[0].result != Some(FileResult::Written) {
        return Ok(Err(format!("undo: {:?}", undone.files[0].result)));
    }
    let after = sorted(library.track_tags(track.id)?);
    if after != before {
        return Ok(Err(format!("tags were {before:?}, are {after:?}")));
    }
    if library.embedded_cover(track.id)? != cover_before {
        return Ok(Err("the cover is not the one it had".into()));
    }
    Ok(Ok(()))
}

fn restore(value: &Option<String>) -> FieldEdit<String> {
    value.clone().map_or(FieldEdit::Clear, FieldEdit::Set)
}

fn set(values: &[&str]) -> FieldEdit<Vec<String>> {
    FieldEdit::Set(values.iter().map(|value| value.to_string()).collect())
}

fn restore_values(values: Option<&Vec<String>>) -> FieldEdit<Vec<String>> {
    values.cloned().map_or(FieldEdit::Clear, FieldEdit::Set)
}

/// A track's tags outside the nine fields, by key.
fn others(entries: &[TagEntry]) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in entries {
        if !FIELD_KEYS
            .iter()
            .any(|key| key.eq_ignore_ascii_case(&entry.key))
        {
            map.entry(entry.key.clone())
                .or_default()
                .push(entry.value.clone());
        }
    }
    map
}

fn check(library: &Library, before: &Track, step: &str) -> Result<(), vcmp_library::Error> {
    let after = library.track(before.id)?.expect("the track stays");
    if after.duration != before.duration {
        println!(
            "{step}: {}: duration {:?} became {:?}",
            name(before),
            before.duration,
            after.duration
        );
    }
    Ok(())
}

fn name(track: &Track) -> String {
    track
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn cover() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(300, 300, image::Rgb([200, 40, 90]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .expect("encoding a JPEG in memory");
    bytes.into_inner()
}
