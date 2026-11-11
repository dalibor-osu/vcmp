# VCMP server

How `crates/vcmp-server` serves a music library to the VCMP app over HTTP, and how
to run it. The agreed decisions are in the
[server spec](specs/2026-11-11-vcmp-streaming-server.md); what is built so far is in
[impl/2026-11-11-vcmp-streaming-server.md](impl/2026-11-11-vcmp-streaming-server.md).
Keep this file in sync with the crate.

The app can't connect to a server yet: the server, its API and its image exist, and
the app's side (mirroring the catalogue, playing server tracks, the admin controls)
is the next part of the work.

## What it is

- **The app's library behind HTTP.** The server opens a `vcmp-library` `Library` on
  its own database and scans, groups, re-links and writes tags with the same code as
  the app ([library.md](library.md)), so a server library behaves like a local one.
- **No user state.** Playlists, favourites, labels and play history stay in each
  app; the server only serves the catalogue, artwork and audio.
- **Two tokens.** A user token browses and plays; an admin token also manages the
  music folders, rescans, edits tags and covers, and replaces tokens.
- **Plain HTTP.** HTTPS comes from a reverse proxy in front of it (see
  [Behind nginx](#behind-nginx)).
- **Linux, x64 and ARM64.** The crate builds and its tests run everywhere the
  workspace does, but only Linux builds are planned for release.

The server never depends on the app, Slint, the playback engine or an audio device.

## Running it

### Docker

The image is built from `packaging/server/Dockerfile` (no image is published yet):

```bash
docker build -f packaging/server/Dockerfile -t vcmp-server .
```

A compose file next to nginx on the same host:

```yaml
services:
  vcmp-server:
    image: vcmp-server
    restart: unless-stopped
    ports:
      - "127.0.0.1:8268:8268"
    volumes:
      - vcmp-data:/data
      - /srv/music:/music:ro
    environment:
      VCMP_SERVER_NAME: Home
volumes:
  vcmp-data:
```

- The port is published on 127.0.0.1 only, so nothing but nginx reaches the plain
  HTTP port.
- `/music` is mounted read-only. Mount it writable (drop `:ro`) only when admins
  should edit tags from the app; the handshake tells the app whether the server can
  write.
- `/data` holds the database, the artwork cache, the server id and the generated
  tokens. Back it up: a server that loses it gets a new id, and every app has to add
  it again.
- The image runs as user 10001. A host folder mounted at `/data` must be writable by
  that user, or run the container with `--user` set to its owner.

The tokens to give the app:

```bash
docker compose exec vcmp-server vcmp-server tokens
```

### As a program

`cargo build --release -p vcmp-server` builds `target/release/vcmp-server`. A
systemd unit:

```ini
[Unit]
Description=VCMP server
After=network-online.target

[Service]
User=vcmp
Environment=VCMP_SERVER_DATA=/var/lib/vcmp-server
Environment=VCMP_SERVER_MUSIC=/srv/music
Environment=VCMP_SERVER_LISTEN=127.0.0.1:8268
ExecStart=/usr/local/bin/vcmp-server serve
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Run `vcmp-server tokens` with the same environment to see the tokens.

### Commands

| Command | Does |
|---|---|
| `vcmp-server` or `vcmp-server serve` | Scans and serves until Ctrl+C or SIGTERM (what `docker stop` and systemd send) |
| `vcmp-server tokens` | Prints the server id and both tokens, made first when the data folder has none |
| `vcmp-server new-token user` (or `admin`) | Replaces a stored token and prints it; a running server takes it within 2 seconds |

`--config <file>` names a configuration file for any of them.

## Configuration

A TOML file (`--config <file>` or `VCMP_SERVER_CONFIG`) and environment variables;
the environment wins. What the configuration sets, it owns: a token it sets can't be
replaced from the app or by `new-token`, and a music folder it names can't be removed
from the app.

| Variable | File key | Default | Meaning |
|---|---|---|---|
| `VCMP_SERVER_NAME` | `name` | `VCMP server` | The name the app shows |
| `VCMP_SERVER_LISTEN` | `listen` | `0.0.0.0:8268` | Address and port; 8268 because the app's AI agent server has 8267 |
| `VCMP_SERVER_DATA` | `data` | `vcmp-server-data` in the working folder | Database, artwork cache, tokens |
| `VCMP_SERVER_MUSIC` | `music` (a list) | none | Music folders added at every start, always with subfolders; several separated by `:` |
| `VCMP_SERVER_USER_TOKEN` | `user_token` | generated | At least 16 visible ASCII characters |
| `VCMP_SERVER_ADMIN_TOKEN` | `admin_token` | generated | The same, and not the user token |
| `VCMP_SERVER_USER_TOKEN_FILE`, `VCMP_SERVER_ADMIN_TOKEN_FILE` | | | A file holding the token, such as a Docker secret |
| `VCMP_SERVER_RESCAN_MINUTES` | `rescan_minutes` | 60 | Minutes between rescans; 0 turns them off |

`RUST_LOG` sets the log level (`info` by default). Tokens never go into the log.

## Behind nginx

The owner's setup: nginx ends HTTPS and passes requests to the server on
127.0.0.1:8268; Cloudflare manages DNS and the certificate, and doesn't proxy the
traffic.

```nginx
server {
    listen 443 ssl;
    http2 on;
    server_name music.example.com;

    ssl_certificate     /etc/ssl/music.example.com/fullchain.pem;
    ssl_certificate_key /etc/ssl/music.example.com/privkey.pem;

    # Tag edits carry covers.
    client_max_body_size 64m;

    location / {
        proxy_pass http://127.0.0.1:8268;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        # Audio goes straight through instead of being buffered to disk.
        proxy_buffering off;
        proxy_read_timeout 1h;
    }
}
```

- **The certificate must be publicly trusted.** With Cloudflare on DNS only, the app
  talks to nginx directly. A Cloudflare Origin CA certificate is trusted only by
  Cloudflare's own proxy, so the app would refuse it; a Let's Encrypt certificate
  obtained through Cloudflare's DNS (certbot's `dns-cloudflare` plugin, for
  example) works.
- **Proxying through Cloudflare** is not needed. If it is turned on, Cloudflare's
  terms for large media and its limits apply; every answer that needs a token says
  `Cache-Control: private, no-store`, so Cloudflare doesn't cache it.

## Security

| Check | Why |
|---|---|
| Every request but `/health` needs `Authorization: Bearer <token>`, else 401 | The library is reachable from the internet. Tokens are 32 random bytes as hex; configured ones need at least 16 characters |
| The token is never taken from a URL, and never logged | URLs end up in proxy logs and browser history |
| The comparison takes the same time wherever the first difference is | Against guessing a token byte by byte |
| Admin calls with the user token get 403 | Only the admin manages folders, writes tags and replaces tokens |
| Answers that need a token carry `Cache-Control: private, no-store` | A proxy or CDN never keeps audio or the library for others |
| Audio is served by track id only, artwork by a key checked to be a hash and an extension | Nothing a client sends becomes a file path |
| Paths in the feed are relative to their music folder, the scan report gives users only file and folder names; whole paths only for admins | The server's file system stays private |
| The music can be mounted read-only | Tag edits are then refused for every file, whatever an admin token asks |

The stored tokens are in the database in the data folder; anyone who can read that
folder can read them.

## The API

The paths, bodies and error codes are `crates/vcmp-protocol`, which the app will use
too. Errors answer with `{"code": ..., "message": ...}`; the app shows its own text
per code, and the message is English detail for logs.

| Path | Role | Does |
|---|---|---|
| `GET /health` | none | `ok` |
| `GET /api/v1/handshake` | user | Server id, name, protocol version, the server's version, the token's role, whether it can write tags |
| `GET /api/v1/feed?after_revision=&after_track=&limit=` | user | The tracks that changed after a cursor, oldest first, 500 by default and at most 2,000; `more` says another page waits, `head` is the newest cursor |
| `GET /api/v1/folders` | user | Music folders: id, name, whether configured and reachable; the whole path for admins |
| `POST /api/v1/folders`, `DELETE /api/v1/folders/{id}` | admin | Add a folder (a path as the server sees it), remove one; both rescan |
| `GET /api/v1/scan`, `POST /api/v1/scan` | user, admin | The last scan's report (file and folder names only for users), or start a rescan (202) |
| `GET /api/v1/artwork/{key}`, `GET /api/v1/thumbnails/{key}` | user | An album's cover, or its 400 px JPEG |
| `GET /api/v1/tracks/{id}/audio` | user | The file as it is, with range requests |
| `GET /api/v1/tracks/{id}/cover` | admin | The cover the file holds, for the tag editor |
| `POST /api/v1/tags` | admin | Writes tag edits: groups of tracks, each with the nine fields and a base64 cover |
| `POST /api/v1/tokens/{user,admin}` | admin | Replaces a stored token; 409 for a configured one |

### The change feed

The app mirrors a server's tracks and asks only for what changed
([library.md, Change feed](library.md#change-feed)):

- Every track carries a revision. Triggers give a track the next revision whenever
  something the feed carries changes in it (its tags, file size, modification time,
  availability, folder, album), and give every track of an album the next revision
  when the album's artwork changes. A rescan that changes nothing changes no
  revision.
- The app keeps the cursor of the last track it received, `(revision, id)`, and
  asks for what comes after it. Several tracks can share a revision; the id breaks
  the tie. An empty page with `more: false` means it is up to date.
- Every page also carries the feed's `head`, the newest cursor. An app holding a
  cursor past it is ahead of the server, which happens when the server's data folder
  was restored from a backup: it then syncs again from the start.
- Tracks are never deleted: a file that goes is in the feed again as unavailable,
  and a removed folder's tracks lose their folder.
- The feed carries each track's shown fields, format, size, modification time, added
  date, MusicBrainz IDs, its album's artwork key and its path inside its folder.
  Albums and artists are not in it: the app groups the tracks itself.

### Audio

- `ETag` is `"<size>-<modification time in ms>"` of the file now, the same value the
  feed's size and modification time give (`vcmp_protocol::audio_etag`), so the app
  can tell from its mirror alone that a cached file is stale.
- One range per request (`bytes=a-b`, `bytes=a-`, `bytes=-n`); anything else gets the
  whole file. A range past the end gets 416.
- `If-Range` with another `ETag` gets the whole file with 200: a download is never
  resumed into a file a tag edit rewrote.

## Threads

| Thread | Runs |
|---|---|
| `vcmp-server-library` | Every write to the library: scans, folder changes, tag writes, on a `Library` of its own. Scans asked for while one waits run once. Replacing a token writes its one setting from the request (or the `new-token` process) instead, so it never waits for a scan |
| `vcmp-server-http` | A multi-threaded tokio runtime serving axum; reads run on tokio's blocking pool, each on a pooled read connection (up to 8 kept) |
| `vcmp-library-watch` | The library's file watcher; a quiet 5 seconds after a change asks for a scan |

A scan never holds up requests (reads use connections of their own, in WAL mode),
and library writes never wait on each other's lock; a token's write waits at most
for one of a scan's transactions of 200 files.

## Testing

`crates/vcmp-server/tests/api.rs` starts the server on a free port over copies of
the library's fixtures and talks real HTTP to it, as the app's sync will: the
handshake, the feed paged one track at a time, artwork, audio downloaded whole and
in two ranges, an admin's tag write and the sync that follows, a stale `If-Range`,
folders, scans and tokens (replaced from the app, by the command, and refused when
configured). `crates/vcmp-library/tests/feed.rs` covers the revisions. Unit tests
cover the configuration, the tokens and the range parsing.

## Known limits

- The app can't connect yet; nothing in the app uses the server.
- The image is not published, and was not built when it was written: Docker wasn't
  running. Releases don't contain the server yet.
- Shared tokens: one person can't be shut out alone, only everyone with that token.
- The app polls the feed; there is no push.
- An admin's folder change or tag write waits while a scan runs, since both write on
  the one writer thread.
- Stopping waits for downloads in flight. `docker stop` kills the server after its
  10-second grace period if a download takes longer.
- A configured music folder that doesn't exist at start is skipped with a warning
  and isn't listed. A folder taken out of the configuration stays as an ordinary
  folder an admin can remove.
- Whether the server can write is checked by making and deleting a hidden
  `.vcmp-write-check` file in each music folder, at start and when folders change.
- Repeated wrong tokens are not slowed down or blocked; generated tokens are too long
  to guess, configured ones need at least 16 characters.
- Covers in tag edits travel base64-encoded, a third larger than the image; a request
  is at most 64 MB.
