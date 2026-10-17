//! Checks tag writing against real files without touching them: copies a sample of
//! each format into `target/edit-check/`, then edits every copy twice (set every
//! field and a cover, then put the text fields back and remove the cover) and prints
//! what failed and why. The music directory is only read.
//!
//!     cargo run --release -p vcmp-library --example edit_check -- <music dir> [files per format]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use vcmp_library::{FieldEdit, Library, TagEdit, Track};
use walkdir::WalkDir;

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
    for track in &tracks {
        let set_all = TagEdit {
            title: FieldEdit::Set(format!("{} ✓ Příliš žluťoučký", track.title)),
            artist: FieldEdit::Set("Edit check artist".into()),
            album_artist: FieldEdit::Set("Edit check album artist".into()),
            track_number: FieldEdit::Set(track.track_number.unwrap_or(0) + 1),
            year: FieldEdit::Set(2001),
            genre: FieldEdit::Set("Edit check".into()),
            cover: FieldEdit::Set(cover.clone()),
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
            ..TagEdit::default()
        };
        for (step, edit) in [("set", &set_all), ("restore", &restore)] {
            let report =
                library.write_tags(&[(track.id, edit)], &AtomicBool::new(false), |_, _| {})?;
            for failed in &report.failed {
                failures += 1;
                println!("{step} failed: {}: {}", name(track), failed.error);
            }
            if !report.written.is_empty() {
                check(&library, track, step)?;
            }
        }
    }
    println!("{} files, {failures} failed edits", tracks.len());
    Ok(())
}

fn restore(value: &Option<String>) -> FieldEdit<String> {
    value.clone().map_or(FieldEdit::Clear, FieldEdit::Set)
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
