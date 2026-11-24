# Install Starbase with Docker

Starbase is one container, `ghcr.io/domeshifter/starbase`. It holds Starbase Sheets
(spreadsheets) with Starbase Drive (the home screen with your files) at `/`, Starbase Docs
(documents) at `/docs/`, and the small server that keeps your files. It runs on any 64-bit Linux
machine with Docker, Intel/AMD or ARM (a Raspberry Pi 4 or 5 works). Other guides:
[Proxmox](proxmox.md), [TrueNAS SCALE](truenas.md), [Unraid](unraid.md).

The suite was renamed recently: the sign-in screen and the log may still say **StarSheets**, and
the settings still start with `STARSHEETS_`. It is the same thing.

| What | Where |
|---|---|
| Web page | port `8080` in the container |
| Your files (`.xlsx`, `.docx`, versions, accounts) | `/data` in the container |
| Backups (optional, recommended) | `/backups` in the container: best another disk or share |
| User the server runs as | `1000` (named `starbase` inside the container; change with `--user`, see below) |

**`sudo` or not:** the commands below use `sudo` where they need root. If you are already root
(a Proxmox container, a minimal Debian, a NAS shell) and there is no `sudo`, leave the word out:

```sh
sudo chown 1000:1000 ~/starbase/data    # as a normal user
chown 1000:1000 /root/starbase/data     # as root, no sudo
```

## Quick start: `docker run`

Make a folder for your files and one for backups (best on another disk; any folder works to
start with), give both to user 1000, then start Starbase:

```sh
mkdir -p ~/starbase/data ~/starbase/backups
sudo chown 1000:1000 ~/starbase/data ~/starbase/backups
docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
  --memory 1536m --memory-swap 1536m \
  -v ~/starbase/data:/data -v ~/starbase/backups:/backups \
  ghcr.io/domeshifter/starbase:latest
docker logs starbase   # shows the setup code
```

Open `http://<your server>:8080`. Keep `--restart unless-stopped`: the server restarts itself
when the owner moves where files are kept. The `--memory` cap stops one enormous file from
pushing the whole machine into swap; leave it out if you prefer.

Forgot the `chown`? The container stops with `Can't use the folder /data: Permission denied`:
run the `chown`, then `docker start starbase`.

## Docker Compose

Download [docker/compose.yml](../../docker/compose.yml) into an empty folder, then:

```sh
mkdir starbase-data && sudo chown 1000:1000 starbase-data
docker compose up -d
docker compose logs starbase   # shows the setup code
```

The compose file already has the memory cap, and the optional settings written in as comments:
your own setup code, the web address used from outside, and a backups folder. Compose names the
container `<folder>-starbase-1`, so use `docker compose logs starbase` (run in that folder), not
`docker logs starbase`.

## First sign-in

The first visit makes the **owner's** account. The server asks for a **setup code**, so that
nobody else on your network can claim the server first:

- By default a random code is printed in the log on first start: `docker logs starbase`, or
  `docker compose logs starbase` with Compose. Look for `setup code:`.
- Or choose your own with `-e STARSHEETS_PASSWORD="a-long-code-only-you-know"` (12 characters or
  more; the server refuses to start with a shorter one). It isn't needed once the owner exists.

The owner then adds other people (avatar › People), or turns on sign-up. Everyone signs in with a
username and password kept on your server.

### Sign in with your own login service

Already run a sign-in service such as Authelia, Authentik, Keycloak or Pocket ID? Starbase can
use it (OpenID Connect) alongside its own passwords: see
[starsheets/docs/oidc.md](../../starsheets/docs/oidc.md).

## Backups

Starbase backs itself up once a day, encrypted, to the folder mounted as `/backups`. It keeps the
last 7 days, 4 weeks and 12 months.

1. **Mount a backups folder** (the Quick start already does). Best on another disk or a NAS
   share than your files. To add one to a container that is already running, make the folder,
   give it to user 1000, and recreate the container with both `-v` lines (your files stay in the
   data folder):

   ```sh
   mkdir -p /mnt/backup-disk/starbase-backups
   sudo chown 1000:1000 /mnt/backup-disk/starbase-backups
   docker rm -f starbase
   docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
     --memory 1536m --memory-swap 1536m \
     -v ~/starbase/data:/data -v /mnt/backup-disk/starbase-backups:/backups \
     ghcr.io/domeshifter/starbase:latest
   ```

   With Compose: remove the `#` in front of the `/backups` line in `compose.yml`, then
   `docker compose up -d`.
2. **As the owner:** avatar › Backups…, check the folder (`/backups` is filled in), pick the hour
   and press **Set up backups**. The first backup starts straight away (within a minute); after
   that it runs daily at the hour you chose.
3. **Keep the recovery key** (Show recovery key › Copy or Download) somewhere other than this
   server. Backups can't be opened without it.

**Time zone.** The hour is the server's time, and the dialog shows which zone that is, e.g.
"server time (Etc/UTC)". The container uses UTC unless you give it your zone: add
`-e TZ=Europe/London` to the `docker run` (with Compose, `TZ: "Europe/London"` under
`environment:`), using your own zone name. Check the dialog afterwards to see it took.

**Restoring files.** Restore (same dialog) copies the **owner's own** files from a backup into a
new folder, "Restored from backup <date>", so nothing is overwritten. It can't bring back other
people's files; for those, or for everything, use restic below.

### With restic, without Starbase

The backups are a standard [restic](https://restic.net) repository, so restic can open them even
if Starbase is gone:

```sh
restic -r /mnt/backup-disk/starbase-backups snapshots              # list the backups
restic -r /mnt/backup-disk/starbase-backups restore latest --target /tmp/sb
```

restic asks for the password: type the recovery key exactly as shown, dashes included (or set
`RESTIC_PASSWORD='xxxx-xxxx-…'` first). The files land under `/tmp/sb/starbase/<username>/…`,
one folder per person; `/tmp/sb/starbase/` also holds the accounts and versions (hidden
folders).

### Server lost? Rebuild from a backup

The restored `starbase` folder is a complete data folder: use it as the new `/data` and everyone's
accounts, passwords and files come back.

1. On the new machine, install Docker and restic, and reach the backups folder (the disk or
   share).
2. Restore and turn it into the data folder:

   ```sh
   restic -r /mnt/backup-disk/starbase-backups restore latest --target /tmp/sb
   mkdir -p ~/starbase/data
   cp -a /tmp/sb/starbase/. ~/starbase/data/
   sudo chown -R 1000:1000 ~/starbase/data /mnt/backup-disk/starbase-backups
   ```

3. Start Starbase with the same `docker run` as in the Quick start (same `/backups` folder). The
   log says `Sign-in is on (N accounts)`; sign in with your old username and password.
4. The backup settings and recovery key are not part of the backup, so set backups up again:
   avatar › Backups…, same folder, tick **That folder has backups from before: use my recovery
   key**, type the key, **Set up backups**. New backups join the old ones.

(Don't set up a fresh owner on an empty server and use Restore in the app for this: it only
looks for that owner's own folder.)

## Updating

```sh
docker pull ghcr.io/domeshifter/starbase:latest
docker rm -f starbase   # your files stay in the data folder
# then the same docker run as above
```

With Compose: `docker compose pull && docker compose up -d`.

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
| `STARSHEETS_PUBLIC_URL` | The address people use from outside, e.g. `https://office.example.com` |
| `TZ` | Time zone for the backup hour, e.g. `Europe/London` (default UTC) |
| `LOCATION` | How the apps tell people where files are kept ("Saved to …"; default "the server") |
| `KEEP` | How many earlier versions of each file to keep (default 6) |
| `--user 1234:1234` | Run as another user, so the files belong to that user on the host (then `chown` the folders to that user instead of 1000) |
