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

## 3. Choose where your files live

Your files can stay inside the container (simplest: they are then in the container's disk and in
its Proxmox backups), or on another disk or NAS share that the **host** has mounted. For the
second, on the Proxmox host's shell (container number `<ct>`, host folder of your choice):

```sh
mkdir -p /mnt/bigdisk/starbase
chown 101000:101000 /mnt/bigdisk/starbase   # user 1000 inside an unprivileged container
pct set <ct> -mp0 /mnt/bigdisk/starbase,mp=/srv/starbase
pct reboot <ct>
```

(An unprivileged container's user 1000 is user 101000 on the host, hence the `chown`. A NAS share
mounted on the host over SMB/CIFS needs `uid=101000,gid=101000` in its mount options instead.)

## 4. Run Starbase

In the container's console:

```sh
mkdir -p /srv/starbase && chown 1000:1000 /srv/starbase
docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
  -v /srv/starbase:/data ghcr.io/domeshifter/starbase:latest
docker logs starbase   # shows the setup code
```

Open `http://<container address>:8080` and make the owner's account with the setup code. StarDocs
is at `/docs/`.

## Backups

- **Proxmox backups** of the container include your files only if they live inside it (step 3,
  first option). A mount point (`mp0`) is left out unless you tick *Backup* on it.
- **Starbase's own backups** (encrypted, once a day, restorable file by file): mount a second
  folder, e.g. `pct set <ct> -mp1 /mnt/nas-backups/starbase,mp=/srv/starbase-backups`, add
  `-v /srv/starbase-backups:/backups` to the `docker run`, then avatar › Backups… in the app.
  Keep the recovery key it shows somewhere else. Details: [Docker guide, Backups](docker.md#backups).

Updating, settings, and use from outside your home: see the [Docker guide](docker.md).
