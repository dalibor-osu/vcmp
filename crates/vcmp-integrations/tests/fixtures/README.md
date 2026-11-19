# Test fixtures

`github-releases.json` is an answer of GitHub's REST API to
`GET /repos/dalibor-osu/vcmp/releases`, written by hand after the documented release
object (API version 2022-11-28). The releases in it are made up: a draft, a release
flagged as a pre-release, one tagged as a pre-release but not flagged, a fix for an
older line published after the newest release, a tag that is no version, the
newest release, v1.3.0, and v1.4.0, flagged as a pre-release but with a plain
version. v1.3.0 and v1.4.0 carry the archives, `SHA256SUMS.txt` and its signature as
the Release workflow names them. The update check's unit tests (`src/updates.rs`)
read it.
