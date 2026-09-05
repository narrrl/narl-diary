# ~/diary

A private, terminal-themed diary that runs in the browser. One user, one SQLite
file, one binary.

- **Rust + axum** backend, **SQLite** storage, **Svelte 5** frontend.
- Markdown entries with full-text search (SQLite FTS5).
- Drag, drop or paste images, video and audio straight into an entry.
- Share any entry behind an unguessable link — revocable at any time.
- Backs itself up to Proton Drive, end-to-end encrypted, as a registered device.
- Real vim keybindings on the desktop (CodeMirror 6 + vim mode), plain taps on
  mobile.
- The whole frontend is embedded in the release binary, so deploying is copying
  one file.

## Quick start

```sh
cp .env.example .env      # set DIARY_USER, DIARY_PASSWORD and DIARY_SECRET
make build                # builds the frontend, then the release binary
./target/release/narl-diary
```

Then open <http://127.0.0.1:4242>.

`DIARY_SECRET` signs the session cookie; generate one with
`openssl rand -base64 32`. Changing it logs every device out. The database and
uploads live under `DIARY_DATA_DIR` (`./data` by default) — that directory is
the entire backup, and it is exactly what the Proton Drive mirror copies.

### Development

```sh
make dev-api   # cargo run, serving the API on :4242
make dev-web   # vite dev server on :4243, proxying /api to :4242
```

In debug builds the binary reads `web/dist` from disk, so `cargo run` alone also
works once the frontend has been built at least once.

## Keys

The desktop UI is modal. In the browse pane:

| key | action |
| --- | --- |
| `j` / `k` | move down / up |
| `gg` / `G` | first / last entry |
| `Enter`, `l` | open the highlighted entry |
| `o` | new entry, straight into insert mode |
| `i`, `a` | edit the open entry |
| `Esc`, `h`, `q` | back out to the list |
| `/` | full-text search (`n` clears it) |
| `s` / `y` | toggle sharing / copy the share link |
| `x`, `dd` | delete the entry |
| `:` | command line |
| `Ctrl-S` | save from anywhere |
| `?` | help |

Inside the editor the full vim keymap is live — `w`/`b`, `dd`, `ciw`, visual
mode, macros, `:w` to write. `:help` lists every ex command; the useful ones are
`:w`, `:wq`, `:q!`, `:new [yyyy-mm-dd]`, `:date`, `:title`, `:share`, `:link`,
`:upload`, `:media`, `:search`, `:backup`, `:set theme=mocha|green|amber|ice` and
`:set novim`.

On a touch device modal editing is turned off and the same actions are buttons.

## Not losing things

Every change is written to `localStorage` as it is typed and to the server a
couple of seconds after typing stops, so `:w` is a habit rather than a
necessity. If a tab dies mid-entry, opening that entry again restores the draft
and says so — `:e!` throws it away, the way vim handles a swap file.

`:export` downloads the open entry as markdown. `:export!` downloads the whole
diary as a zip: one markdown file per entry named by its day, every embedded
file under its real name, and relative links between the two, so the archive
reads in any markdown viewer without this application.

## Backing up to Proton Drive

The server can mirror itself into Proton Drive, where it appears as a device —
its own sync root, alongside the desktop clients — rather than as a folder
dropped in *My Files*. Everything is encrypted client-side before it leaves the
machine, by the same Rust SDK the Linux client uses, so Proton stores a diary it
cannot read.

It is a third-party client: it identifies itself to Proton as
`external-drive-narl_diary@<version>-alpha` and says so before it asks for
account details. It carries no Proton branding and is not supported by Proton.

Set it up once, interactively, because SRP and 2FA need a human:

```sh
narl-diary proton-login                       # or, in Docker:
docker compose exec -it diary narl-diary proton-login
```

That stores a session — tokens, the mailbox password needed to rebuild the key
chain, and the account key salts — as a `0600` file next to the database. It
sits on the same volume as the diary it protects, and it is enough to read the
account, so the volume is the thing to keep private. Built with
`--features keyring` on a host that has a Secret Service, the session goes to
the OS keyring instead and the file is only a fallback; the container has no
session bus, so there it is always the file.

Afterwards the server resumes on its own and nothing prompts again. Refresh
tokens are single-use, so every rotation is written back immediately — which is
also why two servers must not share one session file.

The mirror is one-way and change-driven: a write marks the diary dirty, and once
it has been quiet for `DIARY_BACKUP_DEBOUNCE_SEC` the mirror runs, with
`DIARY_BACKUP_INTERVAL_MIN` as a backstop. An hour of writing is one backup, not
sixty. The device folder ends up as a copy of the data directory:

```text
narl-diary/          the device — Proton allows only folders in a device root
  data/              the diary's data directory, copied
    RESTORE.txt      what this is, and how to put it back
    diary.db         a VACUUM INTO snapshot — consistent, no write-ahead log
    uploads/<uuid>   every uploaded file, under the name the database knows
```

`diary.db` becomes a new revision each time, so Proton Drive keeps the older
ones and a mistake that was mirrored can still be undone. Uploads are written
once and never rewritten. Restoring is copying the contents of `data/`
back into an empty data directory — no tool in between, which is the point.

| command | what it does |
| --- | --- |
| `narl-diary proton-login` | log in and enable backups |
| `narl-diary proton-status` | account, device, schedule, how much is mirrored |
| `narl-diary backup-now` | mirror once and exit — for cron, or for nerves |
| `narl-diary proton-logout` | forget the session; the mirror stays where it is |

From inside the diary, `:backup` says when the last one finished and `:backup!`
runs one now. A failed backup is loud: it is reported by `:backup`, and the next
tick retries it.

Deleting an entry does not delete it from the mirror unless `DIARY_BACKUP_PRUNE`
is on. A backup that forgets on command is one accident away from being no
backup at all.

## Sharing

`:share` mints a 192-bit random token and copies `https://your-host/s/<token>`.
That page needs no session, renders the entry read-only, and serves only the
media files that entry currently embeds — dropping a picture out of a shared
entry immediately makes it unreachable through the link. `:unshare` destroys the
token, and a new `:share` mints a different one.

## API

Everything except the two share routes requires the session cookie.

| method | path | purpose |
| --- | --- | --- |
| `POST` | `/api/login`, `/api/logout` | session |
| `GET` | `/api/me` | current user |
| `GET`/`POST` | `/api/entries` | list (`?q=` searches) / create |
| `GET`/`PUT`/`DELETE` | `/api/entries/{id}` | read / update / delete |
| `GET` | `/api/export` | every entry and file, as a zip |
| `GET`/`POST` | `/api/backup` | Proton Drive mirror: status / run now |
| `POST`/`DELETE` | `/api/entries/{id}/share` | mint / revoke a share token |
| `GET`/`POST` | `/api/media` | list / upload (multipart) |
| `GET`/`DELETE` | `/api/media/{id}` | serve / delete a file |
| `GET` | `/api/share/{token}` | public: read a shared entry |
| `GET` | `/api/share/{token}/media/{id}` | public: media inside a shared entry |

## Deploying

Put a TLS-terminating reverse proxy in front of it, point it at `DIARY_BIND`,
and set `DIARY_SECURE_COOKIE=1`. Nothing else is needed: no database server, no
runtime dependencies, no build tools on the host.

The app sends its own `Content-Security-Policy`, `Referrer-Policy: no-referrer`,
`X-Content-Type-Options` and `X-Frame-Options`; a proxy that adds its own should
not weaken them. `no-referrer` matters in particular, because a share token
lives in the URL and would otherwise leak to any site a shared entry links to.
Uploaded files are only ever served as types that cannot execute — anything else
is handed back as an opaque download.

### Docker

```sh
cp .env.example .env   # set DIARY_USER, DIARY_PASSWORD, DIARY_SECRET
docker compose up -d --build
```

The image builds the frontend with bun and the binary with cargo, then ships
only the binary on `debian:trixie-slim` (~86 MB), running as an unprivileged
user. `DIARY_BIND` and `DIARY_DATA_DIR` are forced in `compose.yml`, so the
values in `.env` do not have to change; everything else is read from `.env`.
Database and uploads live in the `diary-data` volume and survive recreation.

By default the port is published on `127.0.0.1:4242` only. To serve it through
traefik instead, copy `compose.override.yml.example` to `compose.override.yml`
and adjust the host — it joins the external `narl` network, drops the published
port, and routes to container port 4242. Set `DIARY_SECURE_COOKIE=1` in `.env`
once it is served over HTTPS.
