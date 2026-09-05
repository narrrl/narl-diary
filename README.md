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
`:upload`, `:media`, `:search`, `:set theme=green|amber|ice` and `:set novim`.

On a touch device modal editing is turned off and the same actions are buttons.

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
| `POST`/`DELETE` | `/api/entries/{id}/share` | mint / revoke a share token |
| `GET`/`POST` | `/api/media` | list / upload (multipart) |
| `GET`/`DELETE` | `/api/media/{id}` | serve / delete a file |
| `GET` | `/api/share/{token}` | public: read a shared entry |
| `GET` | `/api/share/{token}/media/{id}` | public: media inside a shared entry |

## Deploying

Put a TLS-terminating reverse proxy in front of it, point it at `DIARY_BIND`,
and set `DIARY_SECURE_COOKIE=1`. Nothing else is needed: no database server, no
runtime dependencies, no build tools on the host.
