//! Times each step of a scan on real files, without touching a database: tag reads
//! per format (time and bytes read), and the artwork pipeline (cover read, decode,
//! resize, encode). Files are only read. Linux only (byte counts come from
//! /proc/self/io).
//!
//!     cargo run --release -p vcmp-library --example scan_profile -- <music dir>

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use walkdir::WalkDir;

fn bytes_read() -> u64 {
    let io = std::fs::read_to_string("/proc/self/io").unwrap_or_default();
    io.lines()
        .find_map(|line| line.strip_prefix("rchar: ")?.trim().parse().ok())
        .unwrap_or(0)
}

#[derive(Default)]
struct Stat {
    count: usize,
    bytes: u64,
    times: Vec<Duration>,
}

impl Stat {
    fn add(&mut self, time: Duration, bytes: u64) {
        self.count += 1;
        self.bytes += bytes;
        self.times.push(time);
    }

    fn print(&mut self, name: &str) {
        self.times.sort();
        let total: Duration = self.times.iter().sum();
        let pick = |q: f64| {
            self.times
                .get(((self.times.len() as f64 - 1.0) * q) as usize)
                .copied()
                .unwrap_or_default()
        };
        println!(
            "{name:<22} n={:<5} total={:>7.2}s  mean={:>6.2}ms  p50={:>6.2}ms  p95={:>7.2}ms  max={:>7.1}ms  bytes/file={:>9}",
            self.count,
            total.as_secs_f64(),
            total.as_secs_f64() * 1000.0 / self.count.max(1) as f64,
            pick(0.5).as_secs_f64() * 1000.0,
            pick(0.95).as_secs_f64() * 1000.0,
            pick(1.0).as_secs_f64() * 1000.0,
            self.bytes / self.count.max(1) as u64,
        );
    }
}

fn main() {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: scan_profile <music dir>"),
    );
    let mut files: Vec<PathBuf> = WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            matches!(ext.as_deref(), Some("mp3" | "flac" | "ogg" | "oga" | "wav"))
        })
        .collect();
    files.sort();

    let mut tag_stats: BTreeMap<String, Stat> = BTreeMap::new();
    for path in &files {
        let ext = path
            .extension()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        let (before, started) = (bytes_read(), Instant::now());
        let ok = Probe::open(path)
            .and_then(|p| {
                Ok(p.options(ParseOptions::new().read_cover_art(false))
                    .guess_file_type()?)
            })
            .and_then(|p| p.read())
            .map(|file| file.properties().duration())
            .is_ok();
        let stat = tag_stats
            .entry(format!("tags {ext}{}", if ok { "" } else { " (failed)" }))
            .or_default();
        stat.add(started.elapsed(), bytes_read() - before);
    }
    for (name, stat) in &mut tag_stats {
        stat.print(name);
    }

    // One file per folder stands in for one album.
    let mut seen = std::collections::HashSet::new();
    let album_files: Vec<&Path> = files
        .iter()
        .filter(|p| seen.insert(p.parent().unwrap().to_path_buf()))
        .map(PathBuf::as_path)
        .collect();
    let (mut read, mut decode, mut resize, mut encode) = (
        Stat::default(),
        Stat::default(),
        Stat::default(),
        Stat::default(),
    );
    let mut sizes = Vec::new();
    for path in album_files {
        let (before, started) = (bytes_read(), Instant::now());
        let cover = Probe::open(path)
            .ok()
            .and_then(|p| p.guess_file_type().ok())
            .and_then(|p| p.read().ok())
            .and_then(|file| {
                file.tags()
                    .iter()
                    .flat_map(|tag| tag.pictures())
                    .next()
                    .map(|picture| picture.data().to_vec())
            });
        read.add(started.elapsed(), bytes_read() - before);
        let Some(data) = cover else { continue };
        let started = Instant::now();
        let Ok(image) = image::load_from_memory(&data) else {
            continue;
        };
        decode.add(started.elapsed(), data.len() as u64);
        sizes.push((image.width(), image.height()));
        let started = Instant::now();
        let thumbnail = image.thumbnail(400, 400).into_rgb8();
        resize.add(started.elapsed(), 0);
        let started = Instant::now();
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 85)
            .encode_image(&thumbnail)
            .unwrap();
        encode.add(started.elapsed(), jpeg.len() as u64);
    }
    read.print("cover: read file");
    decode.print("cover: decode");
    resize.print("cover: resize to 400");
    encode.print("cover: encode jpeg");
    sizes.sort_by_key(|(w, h)| w * h);
    if let (Some(small), Some(large)) = (sizes.first(), sizes.last()) {
        println!(
            "embedded cover sizes: smallest {small:?}, median {:?}, largest {large:?}",
            sizes[sizes.len() / 2]
        );
    }
}
