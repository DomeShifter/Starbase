# Install Starbase on Unraid

Starbase isn't in Community Applications yet, so add it from the ready-made template.

## 1. Add the template

In the Unraid terminal (top right, `>_`):

```sh
curl -fsSL -o /boot/config/plugins/dockerMan/templates-user/my-starbase.xml \
  https://raw.githubusercontent.com/DomeShifter/Starbase/main/docs/install/unraid.xml
```

Then **Docker › Add Container › Template: starbase** (under *User templates*).

## 2. Fill it in

| Field | What to put |
|---|---|
| Web port | `8080`, or any free port |
| Files | where your spreadsheets and documents go, e.g. `/mnt/user/appdata/starbase`, or a share such as `/mnt/user/office` |
| Backups (optional) | a share for Starbase's own backups, ideally on other disks than the files |
| Setup code (optional) | your own code for the first sign-in, 12+ characters; leave empty for a random one |

The template runs Starbase as Unraid's usual `nobody` user (`--user 99:100` under *Extra
Parameters*), so the files belong to `nobody:users` like your other shares, and caps it at
1.5 GB of memory. **Apply**.

## 3. First sign-in

Click the container's icon › **Logs** to see the setup code, then **WebUI**. Make the owner's
account with the code; add other people from your
avatar › People. Starbase Docs is at `/docs/`.

## Backups

Fill in the *Backups* path (container already made without it? Docker › starbase › **Edit**, fill
it in, **Apply**: Unraid recreates the container and your files stay put). Then in the app:
avatar › Backups…, choose the hour, **Set up backups**. The first backup runs straight away, then
daily at that hour, in the time zone Unraid passes to its containers (the dialog shows which).
They are encrypted: keep the recovery key somewhere other than the server. The *Files* folder can
also be copied by your usual Unraid backup (for example the Appdata Backup plugin, if it lives in
appdata). Restoring, and rebuilding on a new server from a backup:
[Docker guide, Backups](docker.md#backups); with this template the restored data folder must
belong to `nobody:users` (`chown -R 99:100 …`).

## Updating

Unraid shows *update ready* on the Docker page when a new `latest` image is out: click it.
Settings and use from outside your home: see the [Docker guide](docker.md).
