# ~/diary

A private, terminal-themed diary that runs in the browser. One user, one SQLite
file, one binary.

- **Rust + axum** backend, **SQLite** storage, **Svelte 5** frontend.
- Markdown entries with full-text search (SQLite FTS5).
- Drag, drop or paste images, video and audio straight into an entry.
- Share any entry behind an unguessable link — revocable at any time.
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
the entire backup.

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
`:upload`, `:media`, `:search`, `:set theme=mocha|green|amber|ice` and `:set novim`.

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
