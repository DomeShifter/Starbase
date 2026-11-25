# Running Starbase Sheets on a server

## The easy way: Docker

See the README: `ghcr.io/domeshifter/starsheets` holds the app and its file server in one
container (port 8080, files in `/data`). Settings (each also works under its older name,
`STARSHEETS_…` instead of `STARBASE_…`; when both are set the `STARBASE_` one wins):

| Setting | What it does |
|---|---|
| `STARBASE_PASSWORD` | Your own setup code for the first (owner's) account, at least 12 characters (the server won't start with a shorter one, or with `change-me`). Without it, a random code is made on first start and printed in the log. Not needed once the owner's account exists |
| `STARBASE_AUTH=off` | No sign-in, for a private network you trust: everyone is a guest |
| `LOCATION` | How the app tells people where files are kept ("Saved to …") |
| `KEEP` | The most earlier versions kept per file (default 100). All versions from the last day are kept, then one an hour for a week, then one a day; past the limit the ones closest together go first. Named versions are always kept |
| `STARBASE_OIDC_*` | Single sign-on through your own sign-in service (Authelia, Authentik, Keycloak…): see [oidc.md](oidc.md) |

**Security settings** (all optional; a server on a home network reached by its address works
without them):

| Setting | What it does |
|---|---|
| `STARBASE_PUBLIC_URL` | The address people use, e.g. `https://sheets.example.com`. Requests whose `Origin` is this address are accepted even when a proxy changes the `Host` header, and the server then only answers to this name, IP addresses, `localhost` and `STARBASE_ALLOWED_HOSTS` (against DNS rebinding) |
| `STARBASE_ALLOWED_HOSTS` | More names the server answers to (comma-separated). Setting it also turns the name check on |
| `STARBASE_TRUSTED_PROXY` | Addresses (or ranges like `172.16.0.0/12`) of your reverse proxy. Only from these are `X-Forwarded-For` (who is asking, for the limits on sign-in tries), `X-Forwarded-Proto` (HTTPS: the sign-in cookie is then marked Secure) and `X-Forwarded-Host` believed |
| `STARBASE_SECURE_COOKIE=1` | Always mark the sign-in cookie Secure (only for servers used through HTTPS only) |

What the server does by itself: a request that changes something (and the live-editing
connection) must come from a page of this server: when the browser says where it came from
(`Origin`), that must match the address it was sent to (or `STARBASE_PUBLIC_URL`), so other
web sites can't act for a signed-in person. A proxy should pass the original `Host` header on
(nginx: `proxy_set_header Host $host;`) or you set `STARBASE_PUBLIC_URL`. Sign-in, sign-up and
setup-code tries are limited per address and per username; nothing is read from a request
before its sender is known to be signed in, apart from small account requests (1 MB at most).

Spreadsheets are ordinary `.xlsx` files, in a folder for each person (`<username>/`) and in
folders as people make them in the app. `.starsheets` holds the file and folder ids, versions,
sharing, accounts and sign-in sessions; each person's bin is `<username>/.bin`.

**Where files are kept.** By default, the top of `/data`. The server's owner can choose any folder
inside `/data` (at first setup, or later from their avatar › Where files are kept). Starbase Sheets then
moves everything there (or switches to Starbase Sheets files already in that folder), writes the choice
to `/data/.starsheets-root` and **restarts itself**, so run the container with a restart policy
(`--restart unless-stopped`, as in the README and `docker/compose.yml`). To offer more folders,
mount a bigger folder as `/data`.

**Backups.** Mount a second disk or share as `/backups` (for example
`- /mnt/usb-disk/starbase-backups:/backups` in compose), then the owner chooses avatar › Backups…
and sets it up. Once a day, at the hour chosen, the server copies every file, version, bin and
account there, encrypted with a recovery key. It keeps the last 7 days, 4 weeks and 12 months.
The settings and the key are kept in `/data/.starsheets-backup.json`. **Keep a copy of the key
somewhere else**, because the backups can't be opened without it. Restore (in the same dialog)
copies files from a backup into a new folder, so nothing is overwritten. "Whose files" chooses
your own, another person's (they come back beside their folder, or into yours, named after them,
if they have no account here) or everyone's. Accounts and versions come back only the restic way.

The backups are a [restic](https://restic.net) repository, so if the server itself is lost, any
restic or rustic program can get everything back, with the recovery key as the password:
`restic -r /path/to/backups restore latest --target /somewhere` gives a folder `starbase` holding
the whole files folder (accounts and versions too). Point a new server's `/data` at it.

## By hand

- The app is a static site (`dist/` from `trunk build --release`). Serve WebAssembly as
  `application/wasm`, compressed; hashed files can be cached forever, `index.html` should not be.
- `starsheets-server` (from `crates/server`, a single program) stores the files. Put it behind the
  same host name as the app under `/api/`, or let it serve the app too with `SITE=path/to/dist`.
  It reads the same settings as the Docker image. A folder used by the older `server/files.py`
  works unchanged.
- Live editing uses a WebSocket at `/api/live`: a reverse proxy must pass the `Upgrade` and
  `Connection` headers through (nginx: `proxy_http_version 1.1; proxy_set_header Upgrade
  $http_upgrade; proxy_set_header Connection "upgrade";`) and allow long-lived connections.
  Without it the app still works and saves the usual way, one tab at a time.
- Use HTTPS (or localhost): browsers only give full clipboard access to secure pages.
