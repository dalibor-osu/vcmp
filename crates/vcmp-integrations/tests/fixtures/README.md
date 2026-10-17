# Test fixtures

`github-releases.json` is an answer of GitHub's REST API to
`GET /repos/dalibor-osu/vcmp/releases`, written by hand after the documented release
object (API version 2022-11-28). The releases in it are made up: a draft, a release
flagged as a pre-release, one tagged as a pre-release but not flagged, a fix for an
older line published after the newest release, a tag that is no version, and the
newest release, v1.3.0. The update check's unit tests (`src/updates.rs`) read it.
