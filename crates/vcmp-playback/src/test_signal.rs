//! The reference signal behind the fixtures in `tests/fixtures` (see
//! `make-fixtures.sh` there): 44.1 kHz stereo, 440 Hz left and 660 Hz right at half
//! amplitude, one second cut into two parts at frame `SPLIT`.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};

pub(crate) const RATE: u32 = 44_100;
pub(crate) const SPLIT: usize = 22_433;
pub(crate) const TOTAL: usize = 44_100;

pub(crate) fn fixture(part: &str, extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("tone-{part}.{extension}"))
}

pub(crate) fn reference(frame: usize, channel: usize) -> f32 {
    let frequency = [440.0, 660.0][channel];
    (0.5 * (TAU * frequency * frame as f64 / f64::from(RATE)).sin()) as f32
}
