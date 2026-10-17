//! Outside-world integrations: OS media controls, Last.fm scrobbling, artist photo
//! and cover art lookup, the update check, and Discord Rich Presence. Must not depend
//! on the UI crate.

pub mod discord;
pub mod media_controls;
pub mod musicbrainz;
pub mod updates;
