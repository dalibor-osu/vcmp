//! Outside-world integrations: OS media controls, the OS reduce-motion setting,
//! Last.fm scrobbling, artist photo and cover art lookup, the update check, and
//! Discord Rich Presence. Must not depend on the UI crate.

pub mod discord;
pub mod lastfm;
pub mod media_controls;
pub mod musicbrainz;
pub mod reduce_motion;
pub mod updates;
