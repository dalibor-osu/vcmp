//! Album artwork in the cache directory: the original image under `artwork/` and a
//! downscaled JPEG under `thumbnails/`, both named by a hash of the image bytes. The
//! cache can be deleted at any time; the next scan rebuilds it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;

use crate::error::io_error;
use crate::{Error, Result};

/// Longest side of a thumbnail, in pixels.
pub const THUMBNAIL_SIZE: u32 = 400;

/// File names checked in an album's folder, in order of preference.
const FOLDER_IMAGE_NAMES: &[&str] = &["cover", "folder", "front", "album"];
const FOLDER_IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Paths of an album's cached artwork.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artwork {
    pub original: PathBuf,
    pub thumbnail: PathBuf,
}

pub(crate) struct ArtworkStore {
    originals: PathBuf,
    thumbnails: PathBuf,
}

impl ArtworkStore {
    pub(crate) fn new(cache_dir: &Path) -> Self {
        Self {
            originals: cache_dir.join("artwork"),
            thumbnails: cache_dir.join("thumbnails"),
        }
    }

    /// The cached files for a stored image, if both are still present.
    pub(crate) fn get(&self, key: &str) -> Option<Artwork> {
        let artwork = self.paths(key);
        (artwork.original.is_file() && artwork.thumbnail.is_file()).then_some(artwork)
    }

    /// Stores an image and its thumbnail; returns the key to keep in the database.
    /// `Ok(None)` means the bytes are not an image VCMP can decode.
    pub(crate) fn store(&self, data: &[u8]) -> Result<Option<String>> {
        let Ok(format) = image::guess_format(data) else {
            return Ok(None);
        };
        let Some(extension) = format.extensions_str().first() else {
            return Ok(None);
        };
        let key = format!("{}.{extension}", &blake3::hash(data).to_hex()[..32]);
        let artwork = self.paths(&key);
        if artwork.thumbnail.is_file() && artwork.original.is_file() {
            return Ok(Some(key));
        }
        let Ok(image) = image::load_from_memory_with_format(data, format) else {
            return Ok(None);
        };

        write_atomically(&artwork.original, data)?;
        let thumbnail = image.thumbnail(THUMBNAIL_SIZE, THUMBNAIL_SIZE).into_rgb8();
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 85)
            .encode_image(&thumbnail)
            .map_err(|error| Error::Io {
                path: artwork.thumbnail.clone(),
                source: std::io::Error::other(error),
            })?;
        write_atomically(&artwork.thumbnail, &jpeg)?;
        Ok(Some(key))
    }

    fn paths(&self, key: &str) -> Artwork {
        let stem = key.split('.').next().unwrap_or(key);
        Artwork {
            original: self.originals.join(key),
            thumbnail: self.thumbnails.join(format!("{stem}.jpg")),
        }
    }
}

/// The preferred cover image in a folder, matching names case-insensitively.
pub(crate) fn folder_image(dir: &Path) -> Option<PathBuf> {
    let entries: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .collect();
    let matches = |path: &Path, name: &str| {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase);
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase);
        stem.as_deref() == Some(name)
            && ext.is_some_and(|ext| FOLDER_IMAGE_EXTENSIONS.contains(&ext.as_str()))
    };
    FOLDER_IMAGE_NAMES
        .iter()
        .find_map(|name| {
            entries
                .iter()
                .find(|path| path.is_file() && matches(path, name))
        })
        .cloned()
}

/// Writes through a uniquely named temporary file, so concurrent writers (threads or
/// processes caching the same image) never see a half-written file.
fn write_atomically(path: &Path, data: &[u8]) -> Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().expect("cache paths have a parent");
    fs::create_dir_all(dir).map_err(io_error(dir))?;
    let unique = WRITES.fetch_add(1, Ordering::Relaxed);
    let partial = path.with_extension(format!("{}-{unique}.partial", std::process::id()));
    fs::write(&partial, data).map_err(io_error(&partial))?;
    fs::rename(&partial, path).map_err(io_error(path))
}

#[cfg(test)]
mod tests {
    use image::ImageFormat;

    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 40, 160]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn stores_original_and_bounded_thumbnail() {
        let cache = tempfile::tempdir().unwrap();
        let store = ArtworkStore::new(cache.path());
        let key = store
            .store(&png(1200, 600))
            .unwrap()
            .expect("png is accepted");
        assert!(key.ends_with(".png"));

        let artwork = store.get(&key).expect("files exist");
        let thumbnail = image::open(&artwork.thumbnail).unwrap();
        assert_eq!(
            (thumbnail.width(), thumbnail.height()),
            (THUMBNAIL_SIZE, THUMBNAIL_SIZE / 2)
        );
    }

    #[test]
    fn same_image_gets_same_key_and_garbage_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let store = ArtworkStore::new(cache.path());
        let data = png(10, 10);
        assert_eq!(store.store(&data).unwrap(), store.store(&data).unwrap());
        assert_eq!(store.store(b"not an image").unwrap(), None);
    }

    #[test]
    fn missing_cache_files_read_as_absent() {
        let cache = tempfile::tempdir().unwrap();
        let store = ArtworkStore::new(cache.path());
        let key = store.store(&png(10, 10)).unwrap().unwrap();
        fs::remove_dir_all(cache.path().join("thumbnails")).unwrap();
        assert_eq!(store.get(&key), None);
    }

    #[test]
    fn folder_image_prefers_cover_and_ignores_case() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Folder.JPG"), b"x").unwrap();
        fs::write(dir.path().join("cover.png"), b"x").unwrap();
        fs::write(dir.path().join("cover.txt"), b"x").unwrap();
        assert_eq!(folder_image(dir.path()), Some(dir.path().join("cover.png")));
        fs::remove_file(dir.path().join("cover.png")).unwrap();
        assert_eq!(
            folder_image(dir.path()),
            Some(dir.path().join("Folder.JPG"))
        );
    }
}
