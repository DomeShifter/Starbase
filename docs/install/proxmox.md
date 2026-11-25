# Install Starbase on Proxmox VE

The simple way: a small Debian container (LXC) that runs Docker, with your files in a folder that
can live on any disk or NAS share the Proxmox host can see. About 1 GB of memory and 8 GB of disk
are plenty for the container itself.

## 1. Make the container

In the Proxmox web UI: **Create CT**, with

| Setting | Value |
|---|---|
| Template | Debian 13 (download it first: your storage › CT Templates › Templates) |
| Unprivileged container | yes |
| Disk | 8 GB |
| CPU / memory | 2 cores, 2048 MB memory, 512 MB swap |
| Network | DHCP, or a fixed address you pick |

Before starting it, under **Options › Features** tick **nesting** (Docker needs it). Then start it
and open its **Console**.

## 2. Install Docker

In the container's console:

```sh
apt update && apt install -y curl
curl -fsSL https://get.docker.com | sh
```

## 3. Choose where your files and backups live

Your files can stay inside the container (simplest: they are then in the container's disk and in
its Proxmox backups), or on another disk or NAS share that the **host** has mounted. For the
second, on the Proxmox host's shell (container number `<ct>`, host folders of your choice; the
backups folder is best on a different disk or share from the files):

```sh
mkdir -p /mnt/bigdisk/starbase /mnt/nas-backups/starbase
chown 101000:101000 /mnt/bigdisk/starbase /mnt/nas-backups/starbase   # user 1000 inside the container
pct set <ct> -mp0 /mnt/bigdisk/starbase,mp=/srv/starbase
pct set <ct> -mp1 /mnt/nas-backups/starbase,mp=/srv/starbase-backups
pct reboot <ct>
```

(An unprivileged container's user 1000 is user 101000 on the host, hence the `chown`. A NAS share
mounted on the host over SMB/CIFS needs `uid=101000,gid=101000` in its mount options instead.)

## 4. Run Starbase

In the container's console (you are root there, so no `sudo`):

```sh
mkdir -p /srv/starbase /srv/starbase-backups
chown 1000:1000 /srv/starbase /srv/starbase-backups
docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
  --memory 1536m --memory-swap 1536m -e TZ=Europe/London \
  -v /srv/starbase:/data -v /srv/starbase-backups:/backups \
  ghcr.io/domeshifter/starbase:latest
docker logs starbase   # shows the setup code
```

Use your own time zone in `TZ` (it sets the hour backups run; without it, UTC). Open
`http://<container address>:8080` and make the owner's account with the setup code. Starbase Docs is at `/docs/`.

## Backups

- **Proxmox backups** of the container include your files only if they live inside it (step 3,
  first option). A mount point (`mp0`) is left out unless you tick *Backup* on it.
- **Starbase's own backups** (encrypted, once a day, restorable file by file): in the app,
  avatar › Backups… › **Set up backups**. The first one runs straight away, then daily at the hour
  you pick. Keep the recovery key it shows somewhere other than this server.
- Started without the `/backups` line? Add the `mp1` mount point (step 3), then
  `docker rm -f starbase` and run the `docker run` from step 4 again: your files stay in
  `/srv/starbase`.
- Restoring, and rebuilding on a new server from a backup:
  [Docker guide, Backups](docker.md#backups).

Updating, settings, and use from outside your home: see the [Docker guide](docker.md).
