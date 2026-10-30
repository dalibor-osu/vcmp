//! Animated pictures: GIF, animated WebP and APNG chosen as an album's cover, an
//! artist's photo or a playlist's picture. The library keeps the original file next
//! to the still JPEG everything else shows, and decodes its frames for the few places
//! that play it.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, Frames, ImageFormat, RgbaImage};

use crate::error::io_error;
use crate::{Artwork, Library, Result};

/// The largest animated file kept as an animation. A bigger one is kept as its first
/// frame only, like any still picture.
pub const MAX_ANIMATION_BYTES: usize = 16 * 1024 * 1024;

/// The most frames decoded from one animation; a longer one plays its first frames.
pub const MAX_FRAMES: usize = 1000;

/// The most memory one decoded animation takes, as RGBA. Frames that would take more
/// are decoded smaller.
pub const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;

/// Extensions of the formats that can move, as the library names kept animations.
pub(crate) const ANIMATED_EXTENSIONS: &[&str] = &["gif", "webp", "png"];

/// One frame of an animation, shown for `delay` (as the file gives it; 0 for files
/// that leave it to the viewer).
#[derive(Clone, Debug)]
pub struct Frame {
    pub image: RgbaImage,
    pub delay: Duration,
}

/// Whether `data` is a picture with more than one frame.
pub fn is_animated(data: &[u8]) -> bool {
    frames(data).is_some_and(|frames| frames.take(2).filter(Result::is_ok).count() == 2)
}

/// The frames of an animated picture, each at most `max_side` pixels on its longest
/// side and all of them within [`MAX_DECODED_BYTES`]. `None` for a still picture or
/// one that can't be decoded.
pub fn decode_frames(data: &[u8], max_side: u32) -> Option<Vec<Frame>> {
    decode_within(data, max_side, MAX_DECODED_BYTES)
}

fn decode_within(data: &[u8], max_side: u32, budget: usize) -> Option<Vec<Frame>> {
    if !is_animated(data) {
        return None;
    }
    let mut side = max_side.max(1);
    let mut decoded: Vec<Frame> = Vec::new();
    for frame in frames(data)?.take(MAX_FRAMES) {
        // A damaged frame ends the animation where it is.
        let Ok(frame) = frame else {
            break;
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay =
            Duration::from_micros(u64::from(numerator) * 1000 / u64::from(denominator.max(1)));
        decoded.push(Frame {
            image: bounded(frame.into_buffer(), side),
            delay,
        });
        // Over the budget, every frame so far shrinks, and the rest follow at the
        // new size.
        while decoded_bytes(&decoded) > budget && side > 16 {
            side = side * 7 / 10;
            for frame in &mut decoded {
                frame.image = bounded(std::mem::take(&mut frame.image), side);
            }
        }
    }
    (decoded.len() > 1).then_some(decoded)
}

/// The first frame of a picture as PNG, for the files to hold in place of an
/// animation.
pub fn first_frame_png(data: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(data).ok()?;
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// Whether the file's format may hold an animation, judged by its extension: GIF,
/// WebP or PNG. Only decoding tells whether it does.
pub fn may_be_animated(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ANIMATED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// The animation kept beside a still picture the library keeps (an artist's photo,
/// a playlist's picture): the file of the same name with a GIF, WebP or PNG
/// extension.
pub fn beside(still: &Path) -> Option<PathBuf> {
    kept_animation(still.parent()?, still.file_stem()?.to_str()?)
}

impl Library {
    /// The animation to play for an album's cover, if it may have one: the one kept
    /// for its still (see [`Library::keep_cover_animation`]), otherwise the cached
    /// original when its format may move (a folder's `cover.gif`). Whether that one
    /// does shows only once it is decoded.
    pub fn cover_animation(&self, artwork: &Artwork) -> Option<PathBuf> {
        artwork
            .thumbnail
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| kept_animation(&self.cover_animation_dir(), stem))
            .or_else(|| may_be_animated(&artwork.original).then(|| artwork.original.clone()))
    }

    /// Keeps `animation` as the moving form of the cover `still`, the first frame the
    /// files hold in its place. An album shows it for as long as its cover is that
    /// still, so a cover changed later, by VCMP or another program, ends it. Returns
    /// whether it was kept: a still picture, or one over [`MAX_ANIMATION_BYTES`], is
    /// not.
    pub fn keep_cover_animation(&self, still: &[u8], animation: &[u8]) -> Result<bool> {
        let Some(extension) = animation_extension(animation) else {
            return Ok(false);
        };
        let dir = self.cover_animation_dir();
        fs::create_dir_all(&dir).map_err(io_error(&dir))?;
        let stem = crate::artwork::key_stem(still);
        let path = dir.join(format!("{stem}.{extension}"));
        fs::write(&path, animation).map_err(io_error(&path))?;
        Ok(true)
    }

    fn cover_animation_dir(&self) -> PathBuf {
        self.images.join("covers")
    }
}

/// An animation kept beside a still picture: the file of the same name with a GIF,
/// WebP or PNG extension in `dir`.
pub(crate) fn kept_animation(dir: &Path, stem: &str) -> Option<PathBuf> {
    ANIMATED_EXTENSIONS
        .iter()
        .map(|extension| dir.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
}

/// The extension an animation is kept under: that of its format, when it is one that
/// moves and within [`MAX_ANIMATION_BYTES`]. `None` keeps the picture still.
pub(crate) fn animation_extension(data: &[u8]) -> Option<&'static str> {
    if data.len() > MAX_ANIMATION_BYTES || !is_animated(data) {
        return None;
    }
    match image::guess_format(data).ok()? {
        ImageFormat::Gif => Some("gif"),
        ImageFormat::WebP => Some("webp"),
        ImageFormat::Png => Some("png"),
        _ => None,
    }
}

fn frames(data: &[u8]) -> Option<Frames<'_>> {
    match image::guess_format(data).ok()? {
        ImageFormat::Gif => Some(GifDecoder::new(Cursor::new(data)).ok()?.into_frames()),
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(data)).ok()?;
            decoder.has_animation().then(|| decoder.into_frames())
        }
        ImageFormat::Png => {
            let decoder = PngDecoder::new(Cursor::new(data)).ok()?;
            if !decoder.is_apng().ok()? {
                return None;
            }
            Some(decoder.apng().ok()?.into_frames())
        }
        _ => None,
    }
}

/// The frame, scaled down to fit `side` pixels if it is larger; never scaled up.
fn bounded(image: RgbaImage, side: u32) -> RgbaImage {
    let (width, height) = image.dimensions();
    if width <= side && height <= side {
        return image;
    }
    let scale = f64::from(side) / f64::from(width.max(height));
    let fit = |length: u32| ((f64::from(length) * scale).round() as u32).max(1);
    image::imageops::resize(&image, fit(width), fit(height), FilterType::Triangle)
}

fn decoded_bytes(frames: &[Frame]) -> usize {
    frames.iter().map(|frame| frame.image.as_raw().len()).sum()
}

/// Test pictures, made in code so the tests show what they hold.
#[cfg(test)]
pub(crate) mod samples {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame, Rgba, RgbaImage};

    /// A GIF of `colours.len()` frames, `side` pixels square, each frame filled with
    /// its colour and shown for `delay_ms`.
    pub(crate) fn gif(side: u32, colours: &[[u8; 3]], delay_ms: u32) -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut data);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for &[r, g, b] in colours {
                let image = RgbaImage::from_pixel(side, side, Rgba([r, g, b, 255]));
                encoder
                    .encode_frame(Frame::from_parts(
                        image,
                        0,
                        0,
                        Delay::from_numer_denom_ms(delay_ms, 1),
                    ))
                    .unwrap();
            }
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::samples::gif;
    use super::*;

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn near(pixel: &image::Rgba<u8>, colour: [u8; 3]) -> bool {
        pixel.0[..3]
            .iter()
            .zip(colour)
            .all(|(a, b)| a.abs_diff(b) < 40)
    }

    #[test]
    fn decodes_every_frame_with_its_delay() {
        let data = gif(40, &[RED, GREEN, BLUE], 70);
        assert!(is_animated(&data));
        let frames = decode_frames(&data, 100).expect("an animation");
        assert_eq!(frames.len(), 3);
        for (frame, colour) in frames.iter().zip([RED, GREEN, BLUE]) {
            assert_eq!(frame.image.dimensions(), (40, 40));
            assert!(near(frame.image.get_pixel(20, 20), colour));
            assert_eq!(frame.delay, Duration::from_millis(70));
        }
    }

    #[test]
    fn frames_shrink_to_the_size_asked_for() {
        let frames = decode_frames(&gif(200, &[RED, GREEN], 100), 50).unwrap();
        assert!(
            frames
                .iter()
                .all(|frame| frame.image.dimensions() == (50, 50))
        );
    }

    #[test]
    fn frames_over_the_memory_budget_are_decoded_smaller() {
        // 12 frames of 100 x 100 take 480,000 bytes as RGBA.
        let colours: Vec<[u8; 3]> = (0..12).map(|i| [i * 20, 0, 0]).collect();
        let budget = 200_000;
        let frames = decode_within(&gif(100, &colours, 50), 100, budget).unwrap();
        assert_eq!(frames.len(), 12);
        assert!(decoded_bytes(&frames) <= budget);
        let (width, height) = frames[0].image.dimensions();
        assert!(width < 100 && width == height);
        assert!(frames.iter().all(|frame| frame.image.width() == width));
    }

    #[test]
    fn a_still_picture_is_not_an_animation() {
        let one_frame = gif(10, &[RED], 100);
        assert!(!is_animated(&one_frame));
        assert!(decode_frames(&one_frame, 100).is_none());
        assert_eq!(animation_extension(&one_frame), None);

        let mut png = Cursor::new(Vec::new());
        image::RgbImage::new(10, 10)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        assert!(!is_animated(png.get_ref()));
        assert!(!is_animated(b"not an image"));
    }

    #[test]
    fn the_first_frame_stands_in_as_png() {
        let png = first_frame_png(&gif(30, &[GREEN, RED], 100)).unwrap();
        assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
        let still = image::load_from_memory(&png).unwrap().into_rgba8();
        assert!(near(still.get_pixel(15, 15), GREEN));
    }

    #[test]
    fn only_animations_within_the_limit_are_kept() {
        let data = gif(10, &[RED, GREEN], 100);
        assert_eq!(animation_extension(&data), Some("gif"));
        let mut padded = data.clone();
        padded.resize(MAX_ANIMATION_BYTES + 1, 0);
        assert_eq!(animation_extension(&padded), None);
    }

    #[test]
    fn formats_that_may_move_are_told_by_extension() {
        assert!(may_be_animated(Path::new("a/cover.GIF")));
        assert!(may_be_animated(Path::new("a/b.webp")));
        assert!(may_be_animated(Path::new("a/b.png")));
        assert!(!may_be_animated(Path::new("a/b.jpg")));
        assert!(!may_be_animated(Path::new("a/b")));
    }
}
