//! Outside-world integrations: OS media controls, the OS reduce-motion setting, the
//! file manager (opening a folder, showing a file in it), the window frame without
//! its title bar on Windows, Last.fm scrobbling, artist photo and cover art lookup,
//! lyrics from LRCLIB, the update check and self-update, and Discord Rich Presence.
//! Must not depend on the UI crate.

pub mod discord;
pub mod fanart;
pub mod file_manager;
pub mod lastfm;
pub mod lrclib;
pub mod media_controls;
pub mod musicbrainz;
pub mod reduce_motion;
pub mod self_update;
pub mod updates;
pub mod window_frame;
