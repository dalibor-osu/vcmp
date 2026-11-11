# VCMP streaming server

Status: agreed
Date: 2026-11-11
Area: a new server crate and its Docker image; vcmp-library (reused by the server),
vcmp-playback (a second track source), vcmp-app (connecting to servers, Settings,
library views), vcmp-integrations; releases and documentation

## Context of the change

VCMP plays music from folders on the computer it runs on. Nothing in it reaches
music over the network:

- The v1 specification
  ([2026-10-17-vcmp-music-player.md](2026-10-17-vcmp-music-player.md)) lists
  "network/DLNA sources" and "syncing the library between devices" as out of scope.
  This change reverses the first of these.
- The playback engine has a seam for other sources, still unused. Queue entries are
  `TrackRef { source, id }` and a `TrackSource` opens an id into decoded audio
  (`crates/vcmp-playback/src/track.rs`, `src/source.rs`;
  [playback.md, Data path](../playback.md#data-path)). `SourceKind` has only `Local`,
  whose id is a file path. `TrackSource::open` runs on the engine thread, which also
  handles commands.
- The Spotify plan ([2026-10-20-lyrics-themes-spotify.md](2026-10-20-lyrics-themes-spotify.md))
  is agreed but not built. It designed two things a server source needs as well: a
  table of remote tracks so playlists can hold non-local entries, and a "prepare"
  hint so a network fetch never happens inside `open`. The server is the first
  non-local source to be built, so this spec says how it relates to both.
- `crates/vcmp-library` has no dependency on the app or on Slint. It owns source
  directories, scanning, tag reading and writing, album and artist grouping, the
  artwork cache and FTS5 search ([library.md](../library.md)). A server written in
  Rust can use it as it is.
- Everything the owner adds to tracks is keyed by local library track ids and kept in
  the app's own database: playlists, favourites, labels, play history, the lyrics
  cache (already keyed by source and id), agents' change sets.
- There is a precedent for a token-protected HTTP server: `crates/vcmp-mcp` serves on
  127.0.0.1 through axum on its own tokio runtime, and every request needs a bearer
  token of 32 random bytes ([mcp.md, Security](../mcp.md#security)). Secrets of
  online accounts go to the OS keyring (Last.fm).
- Releases are unsigned archives per platform built by
  `.github/workflows/release.yml`, or locally in Docker by
  `packaging/release-local.sh`. No container image is published today.

## User request

The owner wants to brainstorm a big new feature for VCMP: a VCMP server. It is a
completely separate app, hosted wherever the user desires: on a VPS, a NAS, a
dedicated server and so on. It allows streaming of music to the VCMP client, which
is the current app. The main points it should respect:

- It is hostable in Docker. That is the preferred way of deployment, but it should
  be installable as an app too.
- It is possibly also written in Rust, but a different language is fine if one is
  more suitable.
- It has some kind of authentication. The idea is a server-configured token, ideally
  a random string of characters that the user just puts into their client together
  with the IP or URL of the server.
- Its library works in a similar way as in the current app.
- There are two kinds of auth, admin and user. As the token could be just a random
  string of characters, the server could handle two of these: one for standard users
  and one for admins. Admin versus user permissions are to be brainstormed.
- The VCMP client supports both the local library and a connection to a VCMP server
  at the same time. The server acts as an additional source of music files, maybe in
  a similar way as adding multiple music folders would.
- Connecting to multiple VCMP servers would be nice to have.

Added during the session: the owner's own use-case is to follow. The server is
hosted behind nginx, which handles HTTPS, and Cloudflare manages the certificates.
The server sends original files only for now, but allowing a smaller lossy stream
in the future would be nice. The spec can be longer, since this is a big change.

## Decisions

### What should the VCMP server speak to its clients?

| Option | What it means |
|---|---|
| Own VCMP API (Recommended) | An HTTP API shaped around VCMP's own library (all tags, album grouping, covers, change sync). Smallest and exact fit; only VCMP can connect. |
| OpenSubsonic-compatible | The server speaks the OpenSubsonic API plus VCMP extras where it falls short. Existing phone apps (Symfonium, DSub) can play from it; its user and library model constrains the design. |
| No own server | VCMP becomes an OpenSubsonic client for existing servers (Navidrome, gonic). Months less work; no VCMP-style library, tokens or admin on the server side. |

**Answer: Own VCMP API**

### What is the VCMP server written in, and where does its code live?

| Option | What it means |
|---|---|
| Rust, this workspace (Recommended) | A new crate next to the app, reusing vcmp-library (scanner, tags, grouping, search), so server and local libraries behave the same; server and app share versions and releases. |
| Rust, own repository | Same reuse through a git dependency on vcmp-library, with its own releases and issues; library changes must be coordinated across two repos. |
| Another language | For example C#/ASP.NET Core or Go. Mature web tooling; scanning, tag reading and album grouping are written a second time and can drift from the app. |

**Answer: Rust, this workspace**

### Two shared tokens identify roles, not people, so the server can't tell users apart. Where do playlists, favourites, labels and play history for server tracks live?

| Option | What it means |
|---|---|
| In each client (Recommended) | The server only serves music (catalogue, audio, covers); your playlists, hearts, labels and history stay in each app's own database as today. Two tokens suffice; nothing follows you to a second computer. |
| Shared on the server | The server keeps one set of playlists, favourites, labels and history that everyone with a token sees and edits. Follows you across devices; several people share one account. |
| Per user on the server | Each person gets an own named token (admin or standard) and the server keeps their state. Syncs across devices and keeps people apart; replaces the two-token idea with user management. |

**Answer: In each client**

### Where do the server's two tokens (user and admin) come from?

| Option | What it means |
|---|---|
| Generated, can be set (Recommended) | The server makes both on first start, keeps them in its data folder and shows them through a command; environment variables or the config file can set them instead. Works out of the box and with Docker secrets; two ways to explain. |
| Operator sets them | Tokens come only from the config file or environment, and the server refuses to start without them. Nothing generated or printed; the operator must make strong random strings. |
| Generated only | The server always generates them and a command replaces them. One way to do it; a compose file can't declare the tokens. |

**Answer: Generated, can be set**

### A user token can browse, search and play the server's library. What can the admin token do on top of that? (multi-select)

| Option | What it means |
|---|---|
| Folders and rescan | Add and remove the server's music folders, start a rescan, read the scan report (skipped files, unreachable folders). Without it, folders are set only in the server's config. |
| Edit tags and covers | The app's tag editor works on server tracks; the server writes its files through the same verified-copy writer. Fix tags from anywhere; the server's music mount must be writable. |
| Replace tokens | Make a new user or admin token from the client. Lock someone out without shell access; a stolen admin token can lock the owner out until they use the server's command. |
| Upload and delete music | Send files from the client into the server's folders and delete tracks there. The server becomes manageable without file access; the riskiest right, and large uploads need resumable transfer. |

**Answer: Folders and rescan, Edit tags and covers, Replace tokens**

### Where does an admin use those rights?

| Option | What it means |
|---|---|
| In the VCMP app (Recommended) | A server connected with the admin token shows its admin controls in the app (Settings, tag editor). One UI stack and no second design; admin needs the desktop app. |
| Web page on the server | The server serves a small admin page in the browser. Works from any device, phone included; a second UI (HTML) to build, design and secure. |
| Command line only | A vcmp-server admin command talks to the server's API. Scriptable and cheap; no graphical admin at all. |

**Answer: In the VCMP app**

### A bearer token sent over plain HTTP can be read on the way. How does the VCMP server get HTTPS?

| Option | What it means |
|---|---|
| Reverse proxy (Recommended) | The server speaks plain HTTP; the docs show Caddy, Traefik and nginx in front of it, and the app warns before sending a token over http to a non-local address. Least code; a port exposed bare to the internet leaks the token. |
| Own certificate files | The server can also load a certificate and key itself. HTTPS without a proxy; renewing the certificate is the operator's job. |
| Automatic certificates | The server gets Let's Encrypt certificates for its domain by itself. Easiest safe setup on a VPS; needs a domain and ports 80/443, and is the most code to maintain. |

**Answer (Other): "Here's my use-case, let's follow it: The app would be hosted behind nginx
that handles HTTPS. I also use cloudflare to manage the certificates."**

### Besides the Docker image, how is the VCMP server delivered for installing as an app?

| Option | What it means |
|---|---|
| Plain binary (Recommended) | A vcmp-server binary in the release archives, like the app's; the docs show a systemd unit and how to start it. Cheapest; the operator sets up the service themselves. |
| Binary plus service packages | Also a .deb with a systemd unit and a Windows service setup. A real install; per-platform packaging to maintain, which the app itself doesn't have yet. |
| Docker image only for now | The first release ships only the image; a binary comes later. Smallest release work; people without Docker must build it themselves. |

**Answer: Plain binary**

### Which systems is the VCMP server built for?

| Option | What it means |
|---|---|
| Linux x64 and ARM64 (Recommended) | Image and binary for both. Covers VPSes, most NAS boxes and a Raspberry Pi; the ARM64 cross-build adds release work. |
| Linux x64 only | Simplest release; ARM NAS boxes and Raspberry Pis are left out. |
| Linux, Windows and macOS | Native server binaries for Windows and macOS too, next to the Linux image. Run it on a home PC without Docker; more targets to build and test while CI is off. |

**Answer: Linux x64 and ARM64**

### What audio does the VCMP server send to the app?

| Option | What it means |
|---|---|
| Original files only (Recommended) | The server sends the file's own bytes with range requests and the app decodes them as it does local files. Bit-exact, gapless and seeking unchanged, almost no server CPU; FLAC on a slow or metered link uses a lot of data. |
| Originals plus transcoding | The app can also ask for a smaller lossy stream at a chosen bitrate. Saves bandwidth away from home; the server needs an encoder (ffmpeg in the image), and gapless and exact seeking get harder for transcoded streams. |

**Answer (Other): "Let's do original files only for now, but it would be nice to allow smaller
lossy stream in the future"**

### Does the app keep server audio on the computer after it was played?

| Option | What it means |
|---|---|
| Bounded cache (Recommended) | Downloaded tracks stay in the cache folder up to a size limit set in Settings, oldest dropped first. Replays cost no bandwidth and gapless is easy; no promise that anything plays offline. |
| No cache | A track is held only while it plays or waits as the next one. Simplest and no disk use; every replay downloads the file again. |
| Cache plus keep offline | As the bounded cache, plus a 'Keep offline' action on albums and playlists that downloads them for use without the server. A laptop works on a plane; downloads, storage and stale files need their own UI. |

**Answer: Bounded cache**

### Where does a VCMP server's music appear in the app? (For Spotify, the earlier spec chose an own sidebar section and rejected merging.)

| Option | What it means |
|---|---|
| Merged into the library (Recommended) | Server albums, artists and songs sit in the same Albums, Artists, Songs and Search pages as local ones, as another music folder would, with a source filter and a small mark. One library; an album you have both locally and on the server shows twice, and some of the grid needs the network to play. |
| Own sidebar section | Each server gets its own group in the sidebar with its Albums, Artists, Songs and Search. Always clear what streams, and consistent with the Spotify plan; two places to look and no shared search. |
| Source switch on pages | The existing pages get a switch at the top: Local, a server, or All. One set of pages, either view on demand; one more control on every page and a remembered state to explain. |

**Answer: Merged into the library**

### How does the app get the server's catalogue (tracks, albums, artists, tags)?

| Option | What it means |
|---|---|
| Mirrored in the app (Recommended) | The app copies the server's catalogue into its own database and then fetches only what changed. Browsing and search stay instant and work offline, and an unreachable server greys its tracks out like an unplugged drive; first sync of a big library takes a while. Merging into the library needs this. |
| Asked live | Every page and search asks the server. Nothing to sync and always current; pages wait on the network and show nothing when the server is away. Only fits an own sidebar section. |

**Answer: Mirrored in the app**

### Playlists, favourites, labels, play history, search, scrobbling and Discord cover server tracks through the mirror. Which features that need the file itself also cover server tracks in the first release? (multi-select)

| Option | What it means |
|---|---|
| Lyrics | The server hands out a track's embedded lyrics and the .lrc file next to it. Without this, server tracks only get lyrics from LRCLIB. |
| Folders page | The server's folder tree shows on the Folders page next to local folders. Helps with badly tagged files; the mirror must carry server paths. |
| AI agents | Agents see server tracks and can propose tag changes for them, written by the server after your review (admin token only). Without this, MCP lists and plays local tracks only. |
| Library check | Duplicates, missing tags and missing covers include server tracks, so a track you have locally and on a server is listed as a duplicate. Without this, the check covers local music only. |

**Answer: Folders page**

### You called several servers nice to have. How many VCMP servers can the app connect to in the first release?

| Option | What it means |
|---|---|
| Several from the start (Recommended) | Settings holds a list of servers, like music folders. The data must carry a server id anyway, so the extra is a list instead of one row; more states to test (one server away, the same album on two). |
| One now, built for more | Every server track carries its server's id, but Settings offers a single connection for now. Smaller UI and test surface; the nice-to-have waits for a later change. |

**Answer: Several from the start**

### In how many steps is the VCMP server feature built and released?

| Option | What it means |
|---|---|
| Streaming first, admin second (Recommended) | Step 1: server (image and binary), both tokens, the mirror, playback with the cache, the Folders page; the server's music folders come from its config. Step 2: the admin rights in the app (folders and rescan, tag editing, replacing tokens). Usable sooner and smaller to review; until step 2 the admin token does nothing visible. |
| Everything in one release | Streaming and all admin rights ship together. One announcement and no half-working admin token; longer until anything can be used, and one very large change to review. |
| Server, then app, then admin | Three steps: the server alone with its API and tests, then the app's connection and playback, then admin. Smallest reviews; the first release is of no use to anyone but a tester. |

**Answer: Everything in one release**

### A connected VCMP server can't be reached (no network, server down). What happens to its music in the app's Albums, Artists and Songs pages?

| Option | What it means |
|---|---|
| Stays visible, greyed (Recommended) | Its albums and songs stay on the pages, drawn as unavailable; tracks already in the audio cache still play, and Settings says the server is away. The library doesn't jump on a flaky connection; the grids hold music that won't play. |
| Hidden until it is back | As the app does for local files that are gone: off the library pages, greyed in playlists. The pages show only what plays; on a flaky connection whole albums vanish and return, and cached tracks can't be browsed to. |

**Answer: Stays visible, greyed**

### Someone adds a VCMP server by a plain http:// address on the public internet (no nginx, no HTTPS), so the token would travel unencrypted. What does the app do?

| Option | What it means |
|---|---|
| Warn, then allow (Recommended) | A note says the token is sent unencrypted; the user can connect anyway. Local and private addresses (localhost, 192.168.x.x, 10.x) don't warn. Protects operators without a proxy; one more dialog to build. |
| Refuse | Public addresses must be https. Safest; a setup the app misjudges as public (a VPN, an unusual network) can't connect at all. |
| No warning | The app connects to whatever address it is given. Nothing to build; a leaked token goes unnoticed. |

**Answer: Warn, then allow**

### In your setup, does Cloudflare proxy the traffic to the VCMP server, or only handle DNS and certificates for nginx?

| Option | What it means |
|---|---|
| DNS and certificates only | Audio goes straight from the app to your nginx. Nothing Cloudflare-specific to plan for. |
| Proxied through Cloudflare | Audio passes through Cloudflare's network, so its terms for large media, its caching and its limits apply and are checked before release. |
| Not decided yet | The spec keeps both possible and lists the Cloudflare checks as open. |

**Answer: DNS and certificates only**

### The spec leaves one decision to you before the app's mirror (workstream 3) is built: how do M3U playlists treat server tracks? (asked during implementation)

| Option | What it means |
|---|---|
| Export skips, import local (Recommended) | Export leaves server tracks out and lists them, as it lists paths a file can't hold; import matches only local tracks. Nothing new to design; a playlist with server tracks exports incomplete. |
| Export as stream URLs | Export writes each server track as its http address, import matches those addresses back to server tracks. Round-trips in VCMP; other players can't play them, since the address needs the token. |
| Import also matches by tags | Export skips server tracks as above, but import's artist/title matching may also pick server tracks when no local track matches. Imports find more; a playlist may silently depend on a server. |

**Answer: Export skips, import local**

## High-level plan

One release, as the owner decided. The workstreams are built in this order, and local
playback stays untouched and tested after each. The Servers entry in Settings, the
only way in, lands with workstream 5, so no half-built feature is reachable before.
The schema migration of workstream 3 is the exception: it takes effect wherever it
is merged (see the risk table for the proposed rule).

**1. Server core.** A `vcmp-server` binary over vcmp-library: its data folder
(database, artwork cache, server id, tokens), configuration from environment and a
config file, both tokens (generated, settable, shown and replaced by a command),
a scan at start, the watcher and a periodic rescan. Depends on nothing.

**2. Protocol.** The HTTP API: handshake, the change feed of tracks, artwork, audio
with range requests, and the admin calls (music folders, rescan and its report, tag
and cover writes, the current cover, replacing tokens). Tested over real HTTP, as
vcmp-mcp's server is. Depends on 1.

**3. Mirror in the app.** A migration of the app's library so tracks can come from a
server, the list of connected servers, the sync that writes the mirror, grouping of
mirrored tracks into albums and artists, and the local-only guards. Depends on 2.

**4. Playback.** A server track source for the engine, the prepare hint, a buffering
state, and the bounded audio cache. Depends on 2 and 3.

**5. App UI.** Settings: the list of servers (address and token, state, remove) and
the cache size. A source mark and filter on Albums, Artists, Songs and Search, server
folder trees on the Folders page, and the greyed state while a server is away.
Depends on 3 and 4.

**6. Admin in the app.** For a server connected with the admin token: its music
folders, rescan and scan report, the tag editor on server tracks (single, batch,
cover), and replacing tokens. Depends on 2 and 5.

**7. Packaging and documentation.** The Docker image for x64 and ARM64, binary
archives for both, the release workflow and `packaging/release-local.sh`, a README
section, a new `docs/server.md`, and updates to library.md, playback.md, ui.md,
development.md, manual-testing.md and CLAUDE.md's tables. Starts after 1, ends last.

## Architecture decisions

### Server

- **The server is vcmp-library with an HTTP front.** It opens a `Library` on its own
  database and cache and reuses scanning, tags, re-linking, soft deletion, artwork
  and the verified-copy tag writer unchanged. Reason: "works like the app" then holds
  by construction, and the owner chose reuse over a second implementation.
- **Tokio and axum, as vcmp-mcp.** The server never depends on vcmp-app, Slint,
  vcmp-playback or an audio device, so the image needs no sound or font libraries.
- **Identity is generated, not derived.** The server makes a random server id on first
  start and keeps it in its database. A track is known to clients as server id plus
  the server's track id, which survives moved files as it does in the app. Reason:
  an address changes (IP, domain, port); references in clients must not.
- **Tokens.** Two bearer tokens of 32 random bytes, as the MCP server's. Precedence:
  environment, then config file, then generated and stored in the data folder. Every
  request needs one, audio and covers included; the comparison is constant-time; a
  token never appears in a URL or a log. The handshake says which role a token has.
- **What the configuration sets, the configuration owns.** A token set by
  environment or config file can't be replaced by a client or by the server's
  command: it would be back at the next start, so both say where to change it
  instead. Generated tokens are replaced by an admin's client or by the command.
  Music folders follow the same rule. They live in the server's database, as the
  app's source directories do; a folder named in the configuration is added at
  every start when missing and can't be removed from a client, while folders an
  admin added from the app can. Reason: a compose file stays the truth for what it
  declares, and an admin still manages the rest from the app.
- **Plain HTTP behind a proxy.** The server has no TLS; the owner's nginx ends HTTPS.
  Answers that need a token carry headers that forbid shared caching, so a proxy or
  CDN never stores audio.
- **The watcher is not trusted alone.** Besides watching, the server rescans on an
  interval and on an admin's request. Reason: file events are unreliable on network
  mounts and in containers.
- **Music is mounted read-only unless tag editing is wanted.** The server reports
  whether it can write, and the app offers the tag editor accordingly.

### Protocol

- **Types are shared.** The request and answer types and a protocol version live in
  code both sides compile. The handshake returns the server id, its name, the
  protocol version and the token's role; the app refuses a version it can't speak
  and says which side to update. Server and app share the workspace version and are
  released from the same tag.
- **The mirror is fed by a change feed of tracks.** Every track change on the server
  (new, re-read, unavailable, back) gets a rising revision; a client asks for
  everything after the revision it holds, in pages. The app polls: on connecting, on
  an interval, and after its own admin edits. A full resync is always possible,
  because the mirror is a cache. Reason: polling is enough for a library that
  changes rarely, and no push channel has to survive proxies.
- **Only tracks are mirrored; the app groups them.** The feed carries each track's
  shown fields, duration, format, added date, the ids grouping uses, its path
  relative to its music folder (for the Folders page) and its album's artwork key.
  Albums and artists are formed in the app by the library's own grouping. Reason:
  server album ids change on regrouping, and one grouping code can't disagree with
  itself. Every tag (`track_tags`) is not mirrored, since agents don't cover server
  tracks.
- **Artwork by key.** The server hands out originals and thumbnails by the artwork
  hash; the app caches them as it caches its own.
- **Audio is the file's bytes** with range requests and a validator (size and
  modification time). A client resumes a download only against the same validator,
  so a file rewritten by a tag edit is fetched again, never spliced.

### App

- **Server tracks are rows of the app's library.** A mirrored track gets a local
  track id and remembers its server and the server's track id. Reason: playlists,
  favourites, labels, play history, search, shuffle keys, scrobbling and Discord are
  all keyed by track id and then work without a second code path. The remote tracks
  table of the Spotify plan is not used for servers; that plan is revisited against
  this model when Spotify is built.
- **Albums never span sources; artists do.** The grouping key includes the source, so
  an album held locally and on a server is two albums (the owner's choice). Artists
  stay unique by name, so one artist page lists albums of every source.
- **Two kinds of "unavailable".** A file gone on the server is mirrored as an
  unavailable track and hidden like a local one. A server that can't be reached is a
  connection state, never written to track rows: its music stays on the pages,
  greyed, and what is in the audio cache still plays.
- **Removing a server keeps its rows,** marked unavailable, as removing a music
  folder does. Connecting the same server again (same server id) restores them with
  the same ids, so playlists, hearts, labels and history survive.
- **Local-only guard.** Everything that opens a file by path takes local tracks only:
  scanning and re-linking, file lyrics, the library check, "show in file manager"
  and the MCP tools that search, list, read tags, propose changes or play. A test
  enforces that these never return or touch a server track. Server tracks still get
  LRCLIB lyrics when that switch is on.
- **Agents still meet server tracks by position.** Playlists and the queue are
  mixed, and `get_playlist`, `get_now_playing`, `edit_queue` and the playlist edits
  work by entry position. Those listings keep a server entry in its place, named
  as a server track the agent can't read tags of, change or play, so positions stay
  the ones the window shows; moving and removing such an entry works. The play tools
  leave server tracks out and name them under `left_out`, as they do for missing
  files. The invariant test covers both.
- **The plain http warning.** Adding a server whose address is `http://` and not
  local or private shows a note that the token is sent unencrypted; the user may
  connect anyway (the owner's choice).
- **Tokens go to the OS keyring,** one entry per server id, as the Last.fm session
  key does. The address and name are ordinary settings.
- **Admin acts through the server.** The tag editor sends its edit for server tracks
  to the server, which validates and writes with `Library::write_tags` and re-reads;
  the app then syncs. Folders are typed as the server sees them. Admin controls show
  only for a connection whose handshake said admin.

### Playback

- **A second source, `server`.** `SourceKind` gains it; the track id names the server
  and its track id, so the saved queue needs no format change.
- **The engine thread never waits on the network.** The source gets a prepare hint
  for the next entry (the Spotify plan's idea) and downloads in the background into
  the cache; a stream without data yet reports buffering instead of blocking, and the
  engine keeps answering commands. A test plays through a slow fake server.
- **Decoding is unchanged.** The cached file is decoded by the same symphonia path as
  a local file, so gapless joins, seeking and bit-exactness are the engine's as
  before.
- **The audio cache is bounded.** Whole files in the cache folder, keyed by server,
  track and validator, least recently played dropped first once the size limit from
  Settings is reached; the playing and the prepared file are never dropped.

### Delivery

- **One image, two architectures,** published to the GitHub container registry of the
  repository, tagged with the version. It runs as a non-root user with two mounts:
  the data folder and the music. A health endpoint needs no token and says nothing
  about the library.
- **Plain binaries** for Linux x64 and ARM64 in the release archives; the docs show
  a systemd unit and an nginx example.

## Weaknesses and risks

| Risk | Likelihood | Cost | Mitigation |
|---|---|---|---|
| One release holds a server, a protocol, a schema migration, a new source, a cache, admin and packaging, while CI is off | High | Delayed release, a change too large to review well | Workstream order with local playback tested after each; nothing reachable before workstream 5; all checks run locally before every merge |
| A code path that opens files misses the local-only guard and meets a server track | Medium | Medium: errors, or agents and the check listing music they can't handle | One accessor for a track's local path that server tracks can't satisfy; the invariant test |
| The migration of the app's library breaks existing libraries | Low | High: lost playlists, hearts, labels | Versioned migration tested on a copy of a real library; a newer schema is already refused by older builds |
| A slow or dropped connection stalls the engine or leaves gaps | Medium | Medium-high | Prepare hint, buffering state, slow-server tests; the cache makes replays local |
| The server's database is lost: a new server id, and every client's references to its tracks are orphaned | Low-medium | High | Document backing up the data folder; later, re-linking mirrored tracks by tags as the scanner does |
| Shared tokens: one person can't be shut out alone, a leaked user token downloads the whole library, a leaked admin token rewrites tags and replaces tokens | Medium | Medium | Replacing tokens; the server's command always wins; read-only music mount unless editing is wanted |
| The app's schema migration (workstream 3) merges into `main` before the feature is done: an ordinary release tagged meanwhile ships it, older builds then refuse that database, and a later change of the model needs a second migration | Medium | Medium | Proposed, for the owner to confirm: the workstreams gather on one integration branch and reach `main` together, as PRs 92 to 97 did; otherwise the migration counts as final once merged |
| Someone runs the server without a proxy and the token crosses the internet in clear | Medium for other operators | High | The docs lead with the nginx setup; the app warns before connecting by plain http to a public address |
| An operator proxies audio through Cloudflare (the owner doesn't: DNS and certificates only): its terms for large media on standard plans, caching, request limits | Low | Medium | No-cache headers on every answer that needs a token; the docs say to keep the music host DNS-only or check the terms |
| An agent is confused by server entries it can see in a playlist or the queue but can't play or edit | Medium | Low | The entries say why; the server's instructions to agents explain it |
| The same album locally and on a server fills the grids twice | High for the owner | Low | The source filter; merging is a later decision |
| First sync of a large library is slow; covers arrive lazily | Medium | Low | Paged feed with progress; thumbnails fetched as pages show them |
| Changes on the server show only after the next poll | Certain | Low | Short interval, sync on connect and after admin edits |
| A seek far into a file not yet downloaded waits | Medium | Low | Fetch that range first; show buffering |
| ARM64 builds and image publishing while the release workflow can't run | Medium | Low-medium | Extend `packaging/release-local.sh` to build both architectures |
| No keyring on a minimal Linux desktop | Low | Medium: a server can't be added | Same dependency as Last.fm; a clear error |
| The Spotify plan now differs from what is built (remote tracks table) | Certain | Low now | Revisit that spec before building Spotify |

**Questions deliberately left open**

- The default cache size, rescan interval and poll interval: chosen during
  implementation, after measuring the owner's library over a real link.
- Whether the workstreams reach `main` together through an integration branch (the
  proposed rule in the risk table): answered by building them all on one branch and
  pull request.
- The codec of the future lossy stream: symphonia has no Opus decoder today.
- Whether covers found online for server albums should later be stored on the server
  for every client.

## Out of scope

- Transcoding and lossy streams. Wanted later; the audio call is versioned so a
  format choice can be added.
- User state on the server: playlists, favourites, labels and history are not synced
  between devices, and there are no per-person tokens or accounts.
- Uploading and deleting music through the server.
- An admin web page, admin commands beyond the token command, the OpenSubsonic API
  and phone apps.
- TLS in the server and automatic certificates.
- "Keep offline" downloads; the cache promises nothing offline.
- For server tracks: lyrics from tags and `.lrc` files, AI agents (MCP) beyond
  seeing them by position, the library check, and M3U export and import (the
  owner's choices: export leaves server tracks out and lists them, import matches
  local tracks only).
- Merging the same album from two sources into one, and syncing between servers.
- Service packages (.deb, a Windows service) and server builds for Windows and macOS.
- A push channel for changes.
- Spotify, which stays agreed and unbuilt.
