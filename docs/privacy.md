# Privacy: what Starbase connects to

**By default, Starbase makes no connection to anyone.** No telemetry, no crash reports, no
usage statistics, no update check, no fonts or scripts fetched from other sites. Everything
below happens only when you (or whoever runs your Starbase server) set it up or ask for it.

## Every outbound connection Starbase can make

| What | Which program | When | Goes to | What it sends |
|---|---|---|---|---|
| Connect to a server | Desktop: Docs, Sheets, Drive | Only after you choose **Connect to a server…** and type an address | The Starbase server you typed in | Your device sign-in token and the files you open or save there |
| Signing in with your own login service (OIDC) | Server | Only when its operator sets `STARBASE_OIDC_*` (see the [Docker guide](install/docker.md)) | The login service the operator named | The standard sign-in requests: discovery, token exchange, key fetch |
| Update check | Desktop: Docs, Sheets, Drive | Only when you turn it on (Docs: **Help › Check for updates**; Sheets and Drive: **File › Settings › Check for updates when Starbase starts**), or each time you click **Check for updates now** | GitHub's API: `api.github.com/repos/DomeShifter/Starbase/releases` (Starbase's own releases, never WordCraft's or GridCraft's) | An ordinary web request: no files and no personal data, only what any web request carries, such as your IP address. [GitHub's privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement) |

The update check is **off** on a new install. Turning it on in one Starbase desktop program
turns it on for all of them on that computer (the setting is kept in `updates.json` in
Starbase's settings folder: `~/.config/starbase`, or `%APPDATA%\starbase` on Windows). When it
finds a newer release it tells you and offers the download; nothing is installed without your
say-so.

## What never connects out

- **The web apps** (Sheets, Docs and Drive in a browser) talk only to the Starbase server that
  served them. They never check for updates: the server, and its Docker image, are updated by
  the person who runs it.
- **The server** stores your files on the machine it runs on. It contacts other systems only
  when its operator sets that up (a login service, as above; backups go to a folder the
  operator chooses).
- **Spelling and language data** are built into the programs; nothing is downloaded. (The
  language-pack downloader in the WordCraft engine is not switched on in Starbase.)

## Connections that stay on your own computer

These never leave the machine, and happen only when you use the feature:

- **Read aloud** (Docs, desktop) talks to the computer's own speech service.
- **Citations from Zotero** (Docs, desktop) talk to the Zotero app running on the same
  computer (its local port), if you have it.
- **Links** in a document or sheet open in your browser only when you click them (Sheets asks
  first).

## Checking it yourself

The update checks live in `stardocs/app/src/updates.rs`, `starsheets/app/src/updates.rs` and
`starsheets/crates/connect` (`update_check_on`). Each program has a test that launching it
starts no update check (`no_update_check_starts_at_launch`).

If you find Starbase connecting anywhere this page doesn't list, please
[open an issue](https://github.com/DomeShifter/Starbase/issues): it is a bug.
