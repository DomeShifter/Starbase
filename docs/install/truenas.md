# Install Starbase on TrueNAS SCALE

For TrueNAS SCALE 24.10 (Electric Eel) or later, as a **Custom App** from YAML. Your files are
ordinary `.xlsx` and `.docx` in a dataset, so TrueNAS snapshots and replication cover them.

## 1. Make the datasets

**Datasets › Add Dataset**, under the pool you use for apps:

| Dataset | For | Preset |
|---|---|---|
| `starbase` | your files | Apps |
| `starbase-backups` (optional, ideally on another pool) | Starbase's own backups | Apps |

The *Apps* preset gives the `apps` user (number 568) write access; the app below runs as that user.

## 2. Install the app

**Apps › Discover Apps › ⋮ › Install via YAML**, name it `starbase`, and paste (change `tank` to
your pool's name):

```yaml
services:
  starbase:
    image: ghcr.io/domeshifter/starbase:latest
    user: "568:568"
    ports:
      - "8080:8080"
    volumes:
      - /mnt/tank/starbase:/data
      # - /mnt/tank/starbase-backups:/backups
    environment:
      LOCATION: "the NAS"
      TZ: "Europe/London"   # your time zone: the hour backups run (without it, UTC)
      # Your own setup code instead of a random one in the log (12+ characters):
      # STARBASE_PASSWORD: "a-long-code-only-you-know"
    mem_limit: 1536m
    restart: unless-stopped
```

Port 8080 is taken on your NAS? Change the left number only, e.g. `"8095:8080"`.

## 3. First sign-in

**Apps › starbase › Workloads › Logs** shows the setup code (look for `setup code:`). Open `http://<your NAS>:8080`, make the owner's account
with it, then add other people from your avatar › People. Starbase Docs is at `/docs/`.

## Backups

- **Snapshots** of the `starbase` dataset (Data Protection › Periodic Snapshot Tasks) give you
  earlier copies of every file.
- **Starbase's own backups**: edit the app (**Apps › starbase › Edit**), remove the `#` in front
  of the backups line (pointing at the `starbase-backups` dataset) and save; TrueNAS recreates the
  container and your files stay in the dataset. Then in the app: avatar › Backups… › **Set up
  backups**. The first one runs straight away, then daily at the hour you pick (in the `TZ` time
  zone). They are encrypted; keep the recovery key somewhere off the NAS.
- Restoring, and rebuilding on a new server from a backup: [Docker guide, Backups](docker.md#backups).
  Run `restic` from the TrueNAS shell or another machine that can reach the backups dataset; the
  restored data folder must belong to user 568 (`chown -R 568:568 …`).

## Updating

**Apps › starbase › Update** when TrueNAS offers one, or edit the app and pull the image again.
Settings and use from outside your home: see the [Docker guide](docker.md).
