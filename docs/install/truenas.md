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
      # Your own setup code instead of a random one in the log (12+ characters):
      # STARSHEETS_PASSWORD: "a-long-code-only-you-know"
    restart: unless-stopped
```

Port 8080 is taken on your NAS? Change the left number only, e.g. `"8095:8080"`.

## 3. First sign-in

**Apps › starbase › Workloads › Logs** shows the setup code. Open `http://<your NAS>:8080`, make
the owner's account with it, then add other people from your avatar › People. StarDocs is at
`/docs/`.

## Backups

- **Snapshots** of the `starbase` dataset (Data Protection › Periodic Snapshot Tasks) give you
  earlier copies of every file.
- **Starbase's own backups**: remove the `#` in front of the backups line above (and point it at
  the `starbase-backups` dataset), then avatar › Backups… in the app. They are encrypted; keep the
  recovery key somewhere off the NAS. Details: [Docker guide, Backups](docker.md#backups).

## Updating

**Apps › starbase › Update** when TrueNAS offers one, or edit the app and pull the image again.
Settings and use from outside your home: see the [Docker guide](docker.md).
