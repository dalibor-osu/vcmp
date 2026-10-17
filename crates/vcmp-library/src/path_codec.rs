//! Paths are stored as their platform-native bytes, so file names that are not valid
//! UTF-8 (common in older Linux libraries) survive the round trip through SQLite.

use std::path::{Path, PathBuf};

#[cfg(unix)]
pub(crate) fn encode(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
pub(crate) fn decode(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(windows)]
pub(crate) fn encode(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
pub(crate) fn decode(bytes: &[u8]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    PathBuf::from(std::ffi::OsString::from_wide(&wide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_paths() {
        let path = Path::new("/music/Dvořák/01 Largo.flac");
        assert_eq!(decode(&encode(path)), path);
    }

    #[cfg(unix)]
    #[test]
    fn round_trips_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/music/caf\xe9.mp3"));
        assert_eq!(decode(&encode(path)), path);
    }
}
