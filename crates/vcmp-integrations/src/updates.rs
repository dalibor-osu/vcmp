//! The update check: asks GitHub Releases for VCMP's newest published version and
//! reports it when it is newer than the one running. VCMP doesn't download anything;
//! the release page opens in the browser.
//!
//! The check asks for the list of releases rather than GitHub's "latest" one, and
//! picks the highest version itself: a fix for an older line published after a newer
//! major would otherwise count as the newest. Drafts, pre-releases (flagged as such,
//! or with a pre-release version like `2.0.0-beta.1`) and tags that are no version
//! are skipped.

use std::cmp::Ordering;
use std::thread;
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use crate::musicbrainz::USER_AGENT;

/// The repository's newest releases; ten reach well past any pre-releases.
pub const RELEASES_URL: &str = "https://api.github.com/repos/dalibor-osu/vcmp/releases?per_page=10";
/// A release's page, without its tag.
const RELEASE_PAGE: &str = "https://github.com/dalibor-osu/vcmp/releases/tag/";

/// A published release newer than the running version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The version, without the tag's "v": "1.2.0".
    pub version: String,
    /// The release's page on GitHub, with its notes and installers.
    pub url: String,
}

/// Asks GitHub on a worker thread and hands a release newer than `current` to
/// `on_newer`, on that thread. Finding none, or failing to ask (offline, GitHub's
/// rate limit), is only logged.
pub fn start(current: &str, on_newer: impl FnOnce(Release) + Send + 'static) {
    let current = current.to_owned();
    thread::Builder::new()
        .name("vcmp-update-check".into())
        .spawn(move || match check(&current, fetch) {
            Ok(Some(release)) => {
                log::info!("VCMP {} is available", release.version);
                on_newer(release);
            }
            Ok(None) => log::info!("VCMP {current} is the newest version"),
            Err(error) => log::warn!("could not check for updates: {error}"),
        })
        .expect("spawning the update check thread");
}

/// Fetches the release list with `get` (which returns the status and body) and
/// picks the release newer than `current`, if there is one.
pub fn check(
    current: &str,
    get: impl FnOnce(&str) -> Result<(u16, String), String>,
) -> Result<Option<Release>, String> {
    let current = Version::parse(current)
        .map_err(|error| format!("the running version {current:?} is no version: {error}"))?;
    match get(RELEASES_URL)? {
        (200, body) => newer_release(&body, &current),
        (status, _) => Err(format!("GitHub answered {status}")),
    }
}

/// The newest published release in GitHub's list of releases, if it is newer than
/// `current`.
pub fn newer_release(body: &str, current: &Version) -> Result<Option<Release>, String> {
    let releases: Vec<GitHubRelease> = serde_json::from_str(body)
        .map_err(|error| format!("unexpected answer from GitHub: {error}"))?;
    let newest = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| Some((tag_version(&release.tag_name)?, release.tag_name)))
        .max_by(|(a, _), (b, _)| a.cmp_precedence(b));
    Ok(newest
        .filter(|(version, _)| is_newer(version, current))
        .map(|(version, tag)| Release {
            version: version.to_string(),
            url: format!("{RELEASE_PAGE}{tag}"),
        }))
}

/// Whether `candidate` comes after `current`. Build metadata ("+abc") doesn't count,
/// as semver asks.
pub fn is_newer(candidate: &Version, current: &Version) -> bool {
    candidate.cmp_precedence(current) == Ordering::Greater
}

/// The version a release tag names ("v1.2.0" or "1.2.0"), unless it is a
/// pre-release. Tags are calendar versions, `vYY.MM.PATCH` (README "Versions").
fn tag_version(tag: &str) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix(['v', 'V']).unwrap_or(tag)).ok()?;
    version.pre.is_empty().then_some(version)
}

/// The real network: one GET with the User-Agent GitHub requires.
fn fetch(url: &str) -> Result<(u16, String), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| error.to_string())?;
    Ok((status, body))
}

/// The fields of GitHub's release object the check reads.
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's answer for a repository with a draft, a release flagged as a
    /// pre-release and one tagged as one, a tag that is no version, a fix for an
    /// older line published after the newest release, and that newest release.
    const RELEASES: &str = include_str!("../tests/fixtures/github-releases.json");

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn newer(candidate: &str, current: &str) -> bool {
        is_newer(&version(candidate), &version(current))
    }

    #[test]
    fn versions_compare_by_semver() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.1.1", "0.1.0"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(newer("0.10.0", "0.9.0"), "numbers, not text");
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn calendar_versions_compare_by_date() {
        assert!(newer("26.10.1", "26.10.0"), "a fix that month");
        assert!(newer("26.11.0", "26.10.3"), "the next month");
        assert!(newer("27.1.0", "26.12.4"), "the next year");
        assert!(newer("26.10.0", "0.1.0"), "past the versions before them");
        assert!(!newer("26.9.0", "26.10.0"));
    }

    #[test]
    fn a_release_is_newer_than_its_pre_releases() {
        assert!(newer("1.0.0", "1.0.0-beta.2"));
        assert!(!newer("1.0.0-beta.2", "1.0.0"));
    }

    #[test]
    fn build_metadata_does_not_make_a_version_newer() {
        assert!(!newer("1.0.0+linux", "1.0.0"));
        assert!(!newer("1.0.0", "1.0.0+dev"));
    }

    #[test]
    fn tags_name_versions_with_or_without_a_v() {
        assert_eq!(tag_version("v1.2.0"), Some(version("1.2.0")));
        assert_eq!(tag_version("1.2.0"), Some(version("1.2.0")));
        assert_eq!(tag_version("v2.0.0-rc.1"), None, "a pre-release");
        assert_eq!(tag_version("nightly"), None);
        assert_eq!(tag_version("v1.2"), None);
    }

    #[test]
    fn the_newest_published_release_is_picked() {
        assert_eq!(
            newer_release(RELEASES, &version("0.1.0")).unwrap(),
            Some(Release {
                version: "1.3.0".into(),
                url: "https://github.com/dalibor-osu/vcmp/releases/tag/v1.3.0".into(),
            })
        );
    }

    #[test]
    fn nothing_is_reported_when_up_to_date() {
        assert_eq!(newer_release(RELEASES, &version("1.3.0")).unwrap(), None);
        assert_eq!(
            newer_release(RELEASES, &version("1.4.0")).unwrap(),
            None,
            "a draft or pre-release of a later version doesn't count"
        );
        assert_eq!(newer_release("[]", &version("0.1.0")).unwrap(), None);
    }

    #[test]
    fn an_unexpected_answer_is_an_error() {
        assert!(newer_release(r#"{"message":"Not Found"}"#, &version("0.1.0")).is_err());
        assert!(newer_release("<html>", &version("0.1.0")).is_err());
    }

    #[test]
    fn the_check_asks_github_for_its_releases() {
        let mut asked = None;
        let found = check("1.2.0", |url| {
            asked = Some(url.to_owned());
            Ok((200, RELEASES.to_owned()))
        });
        assert_eq!(found.unwrap().unwrap().version, "1.3.0");
        assert_eq!(asked.as_deref(), Some(RELEASES_URL));
    }

    #[test]
    fn errors_and_other_answers_fail_the_check() {
        assert_eq!(
            check("0.1.0", |_| Err("offline".into())),
            Err("offline".into())
        );
        assert_eq!(
            check("0.1.0", |_| Ok((403, "rate limit exceeded".into()))),
            Err("GitHub answered 403".into())
        );
        assert!(check("not a version", |_| Ok((200, "[]".into()))).is_err());
    }
}
