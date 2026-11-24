# Install Starbase with Docker

Starbase is one container, `ghcr.io/domeshifter/starbase`. It holds StarSheets (spreadsheets and
the home screen, at `/`), StarDocs (documents, at `/docs/`) and the small server that keeps your
files. It runs on any 64-bit Linux machine with Docker, Intel/AMD or ARM (a Raspberry Pi 4 or 5
works). Other guides: [Proxmox](proxmox.md), [TrueNAS SCALE](truenas.md), [Unraid](unraid.md).

| What | Where |
|---|---|
| Web page | port `8080` in the container |
| Your files (`.xlsx`, `.docx`, versions, accounts) | `/data` in the container |
| Backups (optional) | `/backups` in the container: another disk or share |
| User the server runs as | `1000` (change with `--user`, see below) |

## Quick start: `docker run`

```sh
mkdir -p ~/starbase/data && sudo chown 1000:1000 ~/starbase/data
docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
  -v ~/starbase/data:/data ghcr.io/domeshifter/starbase:latest
docker logs starbase   # shows the setup code
```

Open `http://<your server>:8080`. Keep `--restart unless-stopped`: the server restarts itself
when the owner moves where files are kept.

## Docker Compose

Download [docker/compose.yml](../../docker/compose.yml) into an empty folder, then:

```sh
mkdir starbase-data && sudo chown 1000:1000 starbase-data
docker compose up -d
docker compose logs starbase   # shows the setup code
```

The compose file has the optional settings written in as comments: your own setup code,
the web address used from outside, and a backups folder.

## First sign-in

The first visit makes the **owner's** account. The server asks for a **setup code**, so that
nobody else on your network can claim the server first:

- By default a random code is printed in the log on first start (`docker logs starbase`).
- Or choose your own with `-e STARSHEETS_PASSWORD="a-long-code-only-you-know"` (12 characters or
  more; the server refuses to start with a shorter one). It isn't needed once the owner exists.

The owner then adds other people (avatar › People), or turns on sign-up. Everyone signs in with a
username and password kept on your server.

Already run a sign-in service such as Authelia, Authentik or Keycloak? Starbase can use it
(single sign-on, `STARSHEETS_OIDC_*` settings): see
[Single sign-on](../../starsheets/docs/oidc.md).

## Backups

Starbase backs itself up once a day, encrypted, to a folder you mount as `/backups`:

1. Add `-v /mnt/backup-disk/starbase-backups:/backups` (best on another disk or a NAS share than
   your files; the folder must be writable by user 1000).
2. As the owner: avatar › Backups…, turn it on and pick the hour.
3. **Write down the recovery key** and keep it somewhere else. Backups can't be opened without it.

It keeps the last 7 days, 4 weeks and 12 months. Restore (same dialog) copies files from a backup
into a new folder, so nothing is overwritten. The backups are a standard
[restic](https://restic.net) repository, so even without Starbase, `restic -r <folder> restore
latest --target <somewhere>` gets everything back with the recovery key as the password. More:
[starsheets/docs/deploy.md](../../starsheets/docs/deploy.md).

## Updating

```sh
docker pull ghcr.io/domeshifter/starbase:latest
docker rm -f starbase   # your files stay in the data folder
# then the same docker run as above (or: docker compose pull && docker compose up -d)
```

To stay on one version, use a tag such as `ghcr.io/domeshifter/starbase:0.1.0` instead of `latest`.

## From outside your home

Put it behind your reverse proxy with HTTPS (browsers only allow full clipboard use on secure
pages) and set `STARSHEETS_PUBLIC_URL=https://office.example.com`. The proxy must pass WebSockets
through for live editing. Settings for proxies: [deploy.md, Security settings](../../starsheets/docs/deploy.md).

## Other settings

| Setting | What it does |
|---|---|
| `STARSHEETS_PASSWORD` | Your own setup code for the owner's account (12+ characters) |
| `STARSHEETS_AUTH=off` | No sign-in at all, for a private network you trust |
| `STARSHEETS_OIDC_*` | Single sign-on through your own sign-in service ([details](../../starsheets/docs/oidc.md)) |
| `STARSHEETS_PUBLIC_URL` | The address people use from outside, e.g. `https://office.example.com` |
| `LOCATION` | How the apps tell people where files are kept ("Saved to …") |
| `KEEP` | How many earlier versions of each file to keep (default 6) |
| `--user 1234:1234` | Run as another user, so the files belong to that user on the host |
