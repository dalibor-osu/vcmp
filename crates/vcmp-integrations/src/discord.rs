//! Discord Rich Presence: "Listening to VCMP" in the user's Discord status, with the
//! track, artist, a progress bar and the album cover when it is on the web (Discord
//! fetches it from its address; nothing is uploaded).
//!
//! The client runs on its own thread. Discord may not be running, may start later or
//! may quit: the thread retries the connection while there is something to show and
//! never blocks the caller.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

/// The Discord application VCMP's presence belongs to ("Listening to VCMP"). Its ID
/// is public; set `VCMP_DISCORD_APPLICATION_ID` at build time to use another one.
pub const APPLICATION_ID: &str = match option_env!("VCMP_DISCORD_APPLICATION_ID") {
    Some(id) => id,
    None => "1560984330073804883",
};

/// How often a missing Discord is looked for again, and the presence refreshed.
const RETRY: Duration = Duration::from_secs(15);
/// Discord accepts 2 to 128 characters in its text fields.
const MAX_TEXT: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listening {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Where the album cover is on the web (the Cover Art Archive).
    pub cover_url: Option<String>,
    /// How far into the track playback is now.
    pub position: Duration,
    pub duration: Option<Duration>,
}

enum Update {
    Show(Listening, SystemTime),
    Clear,
    Stop,
}

/// Stops the client (and clears the presence) when dropped.
pub struct DiscordPresence {
    updates: Sender<Update>,
    thread: Option<JoinHandle<()>>,
}

impl DiscordPresence {
    /// Starts the client thread. `on_connection` hears (on that thread) when Discord
    /// is reached or lost; the client only looks for Discord while there is a track
    /// to show.
    pub fn start(application_id: &str, on_connection: impl Fn(bool) + Send + 'static) -> Self {
        let (updates, inbox) = mpsc::channel();
        let application_id = application_id.to_string();
        let thread = thread::Builder::new()
            .name("vcmp-discord".into())
            .spawn(move || {
                let mut client: Option<DiscordIpcClient> = None;
                let mut wanted: Option<(Listening, SystemTime)> = None;
                loop {
                    match inbox.recv_timeout(RETRY) {
                        Ok(Update::Show(listening, at)) => wanted = Some((listening, at)),
                        Ok(Update::Clear) => wanted = None,
                        Ok(Update::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    if client.is_none() && wanted.is_some() {
                        client = connect(&application_id);
                        if client.is_some() {
                            on_connection(true);
                        }
                    }
                    if let Some(connected) = client.as_mut() {
                        let result = match &wanted {
                            Some((listening, at)) => {
                                connected.set_activity(activity(listening, *at))
                            }
                            None => connected.clear_activity(),
                        };
                        if let Err(error) = result {
                            log::info!("Discord connection lost: {error}");
                            client = None;
                            on_connection(false);
                        }
                    }
                }
                if let Some(mut connected) = client {
                    let _ = connected.clear_activity();
                    let _ = connected.close();
                }
            })
            .expect("spawning the Discord thread");
        Self {
            updates,
            thread: Some(thread),
        }
    }

    pub fn show(&self, listening: Listening) {
        let _ = self
            .updates
            .send(Update::Show(listening, SystemTime::now()));
    }

    pub fn clear(&self) {
        let _ = self.updates.send(Update::Clear);
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        let _ = self.updates.send(Update::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn connect(application_id: &str) -> Option<DiscordIpcClient> {
    let mut client = DiscordIpcClient::new(application_id);
    match client.connect() {
        Ok(()) => Some(client),
        Err(error) => {
            log::debug!("Discord is not reachable: {error}");
            None
        }
    }
}

fn activity(listening: &Listening, sent_at: SystemTime) -> Activity<'_> {
    // The progress bar runs from when the track would have started.
    let started = sent_at.checked_sub(listening.position).unwrap_or(sent_at);
    let start_ms = started
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut timestamps = Timestamps::new().start(start_ms);
    if let Some(duration) = listening.duration {
        timestamps = timestamps.end(start_ms + duration.as_millis() as i64);
    }

    let mut activity = Activity::new()
        .activity_type(ActivityType::Listening)
        .details(fit(&listening.title))
        .timestamps(timestamps);
    if let Some(artist) = &listening.artist {
        activity = activity.state(fit(artist));
    }
    if let Some(cover) = &listening.cover_url {
        let mut assets = Assets::new().large_image(cover.as_str());
        if let Some(album) = &listening.album {
            assets = assets.large_text(fit(album));
        }
        activity = activity.assets(assets);
    }
    activity
}

/// Fits text into Discord's 2 to 128 characters.
fn fit(text: &str) -> String {
    let mut fitted: String = text.chars().take(MAX_TEXT).collect();
    if text.chars().count() > MAX_TEXT {
        fitted.pop();
        fitted.push('…');
    }
    while fitted.chars().count() < 2 {
        // A zero-width space: invisible, but counts as a character.
        fitted.push('\u{200b}');
    }
    fitted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_fits_discords_limits() {
        assert_eq!(fit("Largo"), "Largo");
        assert_eq!(fit("V").chars().count(), 2);
        let long = "a".repeat(200);
        let fitted = fit(&long);
        assert_eq!(fitted.chars().count(), MAX_TEXT);
        assert!(fitted.ends_with('…'));
    }

    #[test]
    fn a_missing_discord_does_not_block() {
        let presence = DiscordPresence::start("0", |_| {});
        presence.show(Listening {
            title: "Largo".into(),
            artist: None,
            album: None,
            cover_url: None,
            position: Duration::ZERO,
            duration: None,
        });
        presence.clear();
        drop(presence);
    }
}
