# Starbase desktop apps: research for Phases 2–4

Researched 2026-11-24. Report only: nothing in the repo was changed.
Pins looked at: GridCraft `v0.19.1` (our fork, branch `starsheets`), WordCraft `v0.18.1` (our fork, branch `stardocs`).

**Short answer**

| Question | Answer |
|---|---|
| Can we reuse upstream packaging? | Yes, most of it. GridCraft and WordCraft already ship AppImage, deb, rpm, tar, Flatpak, MSI, a portable zip and a DMG, using shell and PowerShell scripts we can copy. Their CI is a shared workflow (`storytold/craft-common-actions`) that we should **not** call directly. |
| One app or three? | **One product, two executables.** Ship one "Starbase" package containing `starsheets` (which also holds the Drive home, as on the web) and `stardocs`, with three menu entries. Do not build one binary: the two engines share an egui fork revision today, but WordCraft `main` has already moved to a newer one. |
| Linux first | AppImage (copy upstream's script almost unchanged) and Flatpak (app ID `io.github.DomeShifter.Starbase`, `--filesystem=xdg-documents`, offline cargo vendoring). |
| Windows | An unsigned WiX MSI plus a portable zip at first. SmartScreen will warn on every release. **Microsoft Artifact Signing does not take UK individuals**, so look at SignPath Foundation (free for open source) first. |
| Mac | Notarisation needs the Apple Developer Program ($99/yr), and the certificate shows a legal name. |
| Server connection | The API is already there (`/api/info`, `/api/files…`, `/api/live` WebSocket). The desktop still needs **bearer device tokens** and a **browser-approval sign-in (loopback redirect)**. No OIDC: the server's rule is "no sign-in through other companies". |

---

## 1. How GridCraft and WordCraft package desktop builds today

### Evidence

- WordCraft's packaging is in `stardocs/upstream/wordcraft/packaging/` and `.github/workflows/release.yml`.
- GridCraft's is the same layout. Checked through the GitHub tree at `v0.19.1`: `packaging/{linux,windows,macos,freebsd,web}` and `.github/workflows/release.yml`.
- Both `release.yml` files are thin callers of `storytold/craft-common-actions/.github/workflows/release.yml@main` (1,463 lines, fetched and read).

### Formats per release (GitHub release assets, sizes from the API)

| Format | Tool / script | GridCraft 0.19.1 | WordCraft 0.18.1 |
|---|---|---|---|
| AppImage (x86_64, aarch64) + `.zsync` | `packaging/linux/package.sh`: appimagetool 1.9.1 + type2-runtime 20251108, both pinned by SHA-256 | 48 MB | 107 MB |
| .deb / .rpm (x86_64, aarch64, riscv64) | nfpm (`nfpm.yaml`) | 51 MB | 114 MB |
| tar.gz (FHS tree) | package.sh | 51 MB | 113 MB |
| Arch .pkg.tar.zst | shared workflow `arch` job | 37 MB | 86 MB |
| Flatpak bundle + OCI image on ghcr.io (signed) | `flatpak-bundle.sh`, `flatpak-oci.sh`, manifests in `packaging/linux/flatpak/` | 37 MB | 81 MB |
| Windows MSI (x64, arm64) | WiX v5 (`gridcraft.wxs`, `package.ps1`) | 40 MB | 91 MB |
| Windows portable zip | package.ps1 | 50 MB | 113 MB |
| macOS universal DMG | `packaging/macos/package.sh`, Info.plist, entitlements, DMG layout | 83 MB | 126 MB |
| FreeBSD tar, web zip, CLI zip | — | — | — |

Every Linux asset has a GPG `.asc`. There is also a `SHA256SUMS.txt` with its own `.asc`.

Starbase would carry both engines, so expect an AppImage of roughly **150 MB** (my estimate: the two upstream sizes added together, minus some shared runtime). The Flatpak would be about **110 MB**. Why WordCraft is twice GridCraft's size is unverified; my guess is embedded fonts and language data.

### Details worth knowing

- **Builds.** CI builds Linux on `ubuntu-22.04` (glibc 2.35) so the binaries run on newer distros. Windows builds with `+crt-static`, so no VC++ redistributable is needed. The deb/rpm dependency lists are careful (Vulkan *or* EGL, Wayland first, X11 recommended).
- **File associations.**
  - Linux: a `.desktop` `MimeType=` line plus a shared-mime-info XML. GridCraft covers xlsx, xlsm, xlsb, ods, csv and tsv. WordCraft covers docx, docm, dotx, doc, odt, rtf, md, tex and html.
  - Windows: in the MSI. The installer is dual-scope (per-user by default, per-machine as an option), and association choices are owned by the app.
  - macOS: in `Info.plist.in`.
- **Auto-update.**
  - The AppImage embeds `gh-releases-zsync|<owner>|<repo>|latest|…zsync` for delta updates. It uses `$GITHUB_REPOSITORY`, so a fork's builds already point at the fork's own releases.
  - In-app updates come from the **`craft-update`** crate (storytold/craft-libs `crates/update`). It reads `api.github.com/repos/<repo>/releases`, detects how the app was installed (AppImage, deb/rpm, Flatpak, MSI, .app) and updates in place.
  - The Flatpak manifest grants `--talk-name=org.freedesktop.Flatpak` and `xdg-data/ArtCraft/versions:ro` for that shared updater.
- **Signing.** The shared workflow *fails* on the release branch unless Apple, Windows (PFX or **Azure Trusted Signing**, the old name for Artifact Signing) and Linux GPG secrets are set. Its macOS job uses `macos-15-xlarge`, a **paid** runner, and is skipped in dry runs for that reason.

### What we can reuse

| Reuse as-is or nearly | Reuse with changes | Don't reuse |
|---|---|---|
| `packaging/linux/package.sh` (AppImage, pinned tools, zsync update info) | `.desktop`, MIME XML and metainfo: rename IDs and texts, and merge them into one package | Calling `craft-common-actions` `release.yml@main`. It is unpinned, storytold-specific, requires signing secrets, and uses paid macOS runners. Copy the parts we need into our own workflow instead. |
| `nfpm.yaml` dependency lists | Flatpak manifest: drop `org.freedesktop.Flatpak` and `ArtCraft/versions`, build both binaries | `craft-update` unpatched. `bundle_id()` hardcodes `ai.storyteller.<id>`, and MSI detection looks for the manufacturer "Learning Machines LLC". |
| The cups-client Flatpak module (printing) | WiX `.wxs` + `install-policy` (dual-scope MSI): new UpgradeCode, product name, two executables | |
| `sign.ps1` (already supports Artifact Signing *and* a PFX) | macOS `package.sh`, Info.plist, DMG layout | |
| Native app glue in `apps/wordcraft/src`: `file_dialogs.rs`, `graphics.rs`, `appimage_env.rs`, `default_apps.rs`, `apple_events.rs`, `print.rs` | (These live in a binary crate, so copy them with attribution, or ask upstream to split out a lib.) | |

---

## 2. One app or three?

### Facts checked

- **Today the two workspaces pin the same stack.**
  - `[patch.crates-io]` sends the whole egui family to `storytold/egui` rev `6d518fc7…` in both `starsheets/Cargo.toml` and `stardocs/Cargo.toml`.
  - Both Cargo.locks resolve egui and eframe 0.37.0, winit 0.30.13 (from the `storytold/winit` rev `a517b384…`), wgpu 30.0.1, accesskit 0.25.1, rfd 0.17.2 and tokio 1.53.2.
  - The only `[patch]` difference is that StarDocs also patches `krilla` to craft-libs `6e03f65b`.
- **It will not stay that way.** `storytold/wordcraft` `main` already pins egui rev `f5025284…`, while `storytold/gridcraft` `main` is still on `6d518fc7…`. Our next WordCraft pin bump would split them.
  - One binary would then need every egui type to come from one crate graph, which Cargo cannot give two different `[patch]` targets for one crate.
  - So one binary is possible today but blocked at the next unaligned bump. The project would then depend on two upstreams moving in step.
- **craft-libs.** craft-libs git revs differ (StarDocs uses `01315d9c`, `6e03f65b`, `762a06a4`, `92d392cf` and `fd5e2fef`; StarSheets uses `6e03f65b`). Different git revs can coexist in one lock, so this is not a blocker on its own.
- **Our app crates are web-only today.**
  - `starsheets/app/src/main.rs` prints "only runs in the browser" on native builds.
  - `home`, `files`, `filebrowser`, `live`, `share`, `account`, `prefs` and `images` are behind `cfg(target_arch = "wasm32")`, and eframe is only a wasm32 dependency.
  - StarDocs has `native_stubs.rs` for its tests only.
  - Native entry points and a storage layer are therefore the main work, not packaging.

### Options

| Option | Good | Bad |
|---|---|---|
| A. One binary (Drive + Sheets + Docs in one eframe app) | One process, one window manager, drag between windows in-process | One workspace that must hold both engines' pins. Blocked by the egui drift above. Every upstream bump is coordinated. One crash takes everything. |
| **B. One package, two executables (recommended)** | Each app keeps its own workspace and upstream cadence. Mirrors the web (StarSheets serves the home; StarDocs is opened from it with `docs/?file=`). Crash isolation. One download, one Flatpak ID, one MSI. | Two processes, each with its own GPU context (more RAM when both are open). Docs are opened by spawning `stardocs --open <path>` or `--server <url> --file <id>`. |
| C. Three separate apps/packages | Simplest per-app CI | Three downloads, three Flathub reviews, three update paths. The Drive home has no natural home. |

### Recommendation: B

- One package called "Starbase" containing `starsheets` (with the Drive home inside it) and `stardocs`.
- Three menu entries:
  - `io.github.DomeShifter.Starbase` (Drive home, which runs `starsheets --home`)
  - `io.github.DomeShifter.Starbase.Sheets`
  - `io.github.DomeShifter.Starbase.Docs`
- Flatpak allows exporting several desktop files as long as each name starts with the app ID.
- Each entry carries its own MIME list, so double-clicking an .xlsx or .docx opens the right executable.
- A third tiny `starbase` launcher binary isn't needed.
- If upstreams ever commit to sharing one egui pin, option A can be revisited without changing the package shape.

---

## 3. Packaging

### 3a. AppImage (Linux, first)

1. Stage an AppDir: `usr/bin/{starsheets,stardocs}`, three `.desktop` files, MIME XML, metainfo and hicolor icons.
   - `AppRun` runs `starsheets --home` by default. It also needs a small dispatcher that picks the right executable when started with a file argument (or as `AppRun stardocs …`).
2. `appimagetool --runtime-file <pinned type2-runtime> -u "gh-releases-zsync|DomeShifter|Starbase|latest|starbase-*-linux-$ARCH.AppImage.zsync"`.
   - This is upstream's exact command. The static runtime means users don't need libfuse2.
3. Build on `ubuntu-22.04` and `ubuntu-22.04-arm`, both free for public repos.
4. Publish `.AppImage`, `.zsync`, `SHA256SUMS` and a GPG signature to GitHub Releases.
5. Updates: zsync delta updates through AppImageUpdate or Gear Lever, plus a small "new version available" notice in the app (see job P4).

### 3b. Flatpak / Flathub

**App ID.** `io.github.DomeShifter.Starbase`.

- Flathub: "Code-hosted apps on GitHub must use the `io.github.` prefix", with at least 4 components, matching a reachable repo. `github.com/DomeShifter/Starbase` qualifies.
- This also avoids putting a personal domain in the repo.

**Rules that matter (docs.flathub.org requirements page):**

- **Builds run offline.** Generate `cargo-sources.json` with `flatpak-cargo-generator.py` from each `Cargo.lock`; it handles the git dependencies (egui/winit forks, craft-libs). That means two lockfiles and two generated files, one module per executable.
  - The engines are fetched by `fetch-upstream.sh`, so the manifest needs `git` sources for both forks at the pinned commits.
  - Fonts and word lists fetched in CI also need to be manifest sources with URL + sha256.
- **"All source available submissions must be built entirely from source code".** No repackaging of our release binaries for Flathub. Upstream's `.bundle.yml` approach (repackaging) is only good for our own `.flatpak` asset.
- **MetaInfo is mandatory.** It must validate, use the same ID, give the licence, and include screenshots and releases.
- **Licences** go in `/app/share/licenses/$FLATPAK_ID`.
- **"Static permissions must be kept to an absolute minimum"**, and portals are preferred.
- **Third-party submissions are rejected** if upstream ships its own Flatpak outside Flathub, and "forks with minimal changes" are rejected.
  - Starbase is its own app with its own UI and server features, so this should be fine.
  - Expect reviewers to ask how it differs from GridCraft and WordCraft. Prepare a paragraph.
- **New apps go to stable**, through a PR to `flathub/flathub` (`new-pr` branch). Review takes days to weeks.

**finish-args (start from upstream's, minus its updater rights):**

```
--socket=wayland --socket=fallback-x11 --share=ipc --device=dri
--share=network                       # connect to a Starbase server
--filesystem=xdg-documents            # "works on ~/Documents" (open recent, save-next-to-original)
--socket=cups --system-talk-name=org.freedesktop.Avahi --filesystem=~/.cups:ro   # printing, as upstream
# NOT: --talk-name=org.freedesktop.Flatpak (linter warning finish-args-flatpak-spawn-access; Flathub does the updating)
# NOT: --filesystem=xdg-data/ArtCraft/versions:ro, home, host
```

Anything outside ~/Documents goes through the FileChooser portal. rfd's xdg-portal backend already does this upstream. Drag and drop goes through the Document portal.

Store the server token in the Secret portal or keyring (`org.freedesktop.secrets`): a `keyring` crate with the Secret Service backend works from inside Flatpak through the portal. Check this in the token job.

**Our own `.flatpak` bundle.** Also attach one to releases so people can install without Flathub. Upstream additionally pushes an OCI remote to ghcr; that can wait.

### 3c. Windows (Phase 3, unsigned at first)

| Option | Notes | Verdict |
|---|---|---|
| **WiX v5 MSI (upstream's)** | Dual-scope, no admin needed per-user, file associations, upgrade codes, tested policy scripts. Copy `gridcraft.wxs` + `install-policy.*`. | **Use** |
| cargo-wix | Wraps WiX 3/4 for a single binary; less control than upstream's tested .wxs | Skip |
| NSIS | Smaller, easy custom UI; association/upgrade logic by hand | Skip |
| Portable zip | Upstream ships one; trivial | Ship alongside |
| winget | Manifest PR to `microsoft/winget-pkgs` (`wingetcreate`); points at the GitHub MSI URL + sha256. Unsigned installers are allowed, but the validation pipeline scans them and can flag new unsigned publishers. | After 1–2 releases |

**SmartScreen, unsigned.**

- A downloaded MSI carries the Mark of the Web. Users see "Windows protected your PC", then "More info", then "Run anyway", and the publisher shows as "Unknown".
- Reputation for unsigned files is per file hash, so **every release starts from zero**.
- Extracting the portable zip with Explorer passes the mark on to the exe.
- Say this plainly on the download page.

**Signing later.** `sign.ps1` already supports both routes below.

| Route | Cost | Name on certificate | Catch |
|---|---|---|---|
| Microsoft **Artifact Signing** (ex Trusted Signing) | Paid Azure subscription, Basic SKU (about $10/month, check the pricing page) | The validated **legal name** of the person or organisation | Quickstart: "Individual developers must be located in the United States or Canada". **UK is only open to organisations**, so a registered legal entity is needed, and its name appears publicly. Validation takes 1–30 business days. SmartScreen reputation still "builds up automatically", so warnings don't vanish on day one. |
| **SignPath Foundation** (free open-source signing) | Free | "SignPath Foundation" | Needs an OSI licence (MIT/Apache: yes), no proprietary parts, already released, MFA for the whole team, verifiable CI builds from the repo, and manual approval of each release. It keeps personal names off the certificate, which suits the "no names" public voice. |
| OV certificate on a hardware token or cloud HSM | About £200–400/yr | Legal name | Worse than both above for us |

**Recommendation:** apply to SignPath Foundation once the first unsigned Windows release is out. Keep Artifact Signing as the fallback if James sets up a company.

### 3d. Mac (Phase 4)

- **Build.** Universal binary (`lipo` of x86_64 + aarch64). The `.app` bundle needs an Info.plist with `CFBundleDocumentTypes`/UTIs for the formats, and file opens arrive through Apple Events (upstream's `apple_events.rs`). Ship it as a DMG (upstream's layout, `generate.py`).
- **Signing and notarisation.**
  - Requires the Apple Developer Program ($99/yr) and a "Developer ID Application" certificate. The certificate shows the account holder's legal name, or an organisation's name if it has a D-U-N-S number.
  - Steps: `codesign --options runtime --timestamp` with `entitlements.plist`, then `xcrun notarytool submit --wait`, then `xcrun stapler staple` on the .app and the DMG.
  - Upstream's `package.sh` already does all of this when the secrets are set.
- **Unsigned.** Gatekeeper blocks the app. On current macOS the user has to go to System Settings › Privacy & Security › "Open Anyway". (Recent macOS removed the old Control-click bypass; I believe this changed in macOS 15, but it is unverified here.) That is acceptable for early testers only.
- **CI.** Use the standard free `macos-15` (arm64) runner, not upstream's paid `macos-15-xlarge`. It is slower but costs nothing on a public repo.

---

## 4. "Connect to a server" from the desktop app

### What the server already offers

The source is `starsheets/crates/server/src/main.rs` and its modules. Everything is under `/api/`.

**Discovery**

- `GET /api/info` returns `{location, auth, signed_in, setup, signup, signup_code, user}`.
- This is a good first call when the user types a server address.

**Accounts**

- `POST /api/login`, `/api/logout`, `/api/setup`, `/api/signup`
- `GET/POST /api/me`
- `/api/users…`, `/api/users/password`

**Files**

- `GET/POST /api/files`, `GET/PUT/DELETE /api/files/<id>` (ETag / `If-Match`; a 409 means it was changed elsewhere)
- `/api/files/<id>/{move,rename,copy,restore,versions/…,sharing,star,colour,thumb,add-sheets,opened}`
- `/api/files/folders`, `/api/folders/…`, `/api/bin`, `/api/shared`, `/api/starred`, `/api/storage`

**Live editing**

- `GET /api/live`: a WebSocket. The protocol types are in the `starsheets/crates/sync` crate, which is already shared with the web app and is native-friendly.

**Auth today**

- Username and password are checked against Argon2 hashes. The session is the `starsheets_session` cookie (HttpOnly, SameSite=Strict, `SESSION_DAYS = 30`), and only a hash of the token is stored server-side.
- `guard.rs` rejects requests whose `Origin` is a different site. "No Origin: not a cross-site browser request" is allowed, so **native clients already pass the guard**.

### What to add

1. **Device tokens.**
   - `POST /api/tokens {name}` returns `{token}`, shown once and stored hashed like sessions.
   - `GET/DELETE /api/tokens` lets people see and revoke them ("This laptop, last used …").
   - The server accepts `Authorization: Bearer <token>` everywhere a session cookie is accepted, **including the `/api/live` WebSocket upgrade** (native WebSocket clients can set headers; browsers can't, which is why the web uses the cookie).
   - Tokens should be long-lived but revocable and listed in account settings.
2. **Sign-in from the desktop. Recommended: browser approval with a loopback redirect, in the style of RFC 8252 but without a full OIDC provider:**
   1. The app listens on `http://127.0.0.1:<random port>/cb`.
   2. It opens `https://<server>/connect?port=<p>&state=<s>&code_challenge=<PKCE>` in the default browser.
   3. The user is already signed in there (or signs in, including any future second factor) and clicks "Allow Starbase on <device>".
   4. The server redirects to the loopback address with a one-time code.
   5. The app posts code + verifier to `POST /api/tokens/exchange` and gets the token.

   - Fallback for a Flatpak or remote session without a browser: a **device-code flow** (RFC 8628 style), where the app shows `ABCD-1234` and the user enters it at `https://<server>/connect`.
   - Simplest stop-gap: a username/password dialog in the app that calls `/api/login` and keeps the cookie. It works today, but the app then handles passwords and gets no per-device revocation. Use it only until (1) and (2) land.
   - **OIDC:** not needed, and against the server's stated rule ("No sign-in through other companies"). The loopback/PKCE design above keeps the door open: if an OIDC option is ever added, the desktop flow stays the same.
3. **Version handshake.** Add `api_version` and the server version to `GET /api/info`, so an older desktop can say "this server is newer, please update" rather than failing at random.
4. **TLS.** The native client uses rustls with the platform roots (`rustls-platform-verifier`). For home servers with self-signed certificates, offer "trust this certificate" pinning by its SHA-256 per server. Never an "ignore errors" switch.
5. **Token storage.** Use the OS keyring (`keyring` crate): Secret Service on Linux (works in Flatpak), Credential Manager on Windows, Keychain on Mac.

### Local mode (no server, no account)

- The Drive home lists a folder tree under `~/Documents`: Starbase's own root, or the whole of Documents, a choice for James.
- Files save to disk a moment after each change, as on the server.
- A file changed outside the app is spotted by mtime and size, and handled the same way as a 409 (a conflicted copy, never an overwrite).
- Recent and starred lists live in `$XDG_DATA_HOME/starbase/` (or `%APPDATA%`), never as hidden files inside Documents.
- Version history in local mode is optional for Phase 2.
- Alternative: run the server's `store.rs` in-process for versions and bin. It is more code to extract, and it writes a `.starsheets/` folder into the chosen folder.

---

## 5. Phase 2 (Linux desktop) job list

Sizes: **S** is under a day, **M** is 1–3 days, **L** is 3+ days of agent work.

| # | Job | Size | Depends on | Notes |
|---|---|---|---|---|
| D1a | StarSheets native entry point: eframe native features (wgpu, wayland, x11, accesskit, clipboard), window, icon, open the file given on the command line; copy the upstream glue (`graphics.rs`, `file_dialogs.rs`, `appimage_env.rs`) with attribution | M | — | |
| D1b | StarDocs native entry point (same as D1a) | M | — | Parallel with D1a |
| D2a | StarSheets storage layer: a `Backend` trait (list, open, save with version check, new, move, rename, bin, versions) with a native HTTP client (ureq or reqwest on a thread) next to the existing wasm `fetch` one. Port `home/`, `files.rs` and `filebrowser.rs` off `web_sys`. | L | D1a | Biggest job |
| D2b | StarDocs: put `server.rs` (Link) behind the same trait shape | M | D1b | Parallel with D2a |
| D3 | Local backend: `~/Documents` tree, autosave, outside-change detection, recent/starred in the XDG data dir. Same trait, both apps. | M | D2a, D2b (the trait) | Can start once the trait is agreed |
| D4 | Drive home opens Docs: spawn `stardocs --open <path>` / `--server <url> --file <id>`, find the sibling executable, check the `$APPDIR` and Flatpak paths | S | D1a, D1b | |
| D5 | Native live editing: `/api/live` over tungstenite with the `sync` crate; reconnect | M | D2a, S1 | |
| S1 | Server: device tokens (`/api/tokens`, Bearer on HTTP + WebSocket), loopback/PKCE `/connect` page + `/api/tokens/exchange`, device-code fallback, `api_version` in `/api/info`; tests | M | — | **Parallel from day one** |
| D6 | Desktop "Connect to a server" dialog: address → `/api/info` → browser approval → token in keyring; TLS certificate pinning prompt; sign out / forget server | M | S1, D2a | |
| P1 | Packaging assets: app IDs (`io.github.DomeShifter.Starbase[.Sheets/.Docs]`), 3 `.desktop` files, MIME XML (copy upstream lists), metainfo with screenshots, hicolor icons, licences/NOTICE | S | — | Parallel |
| P2 | `packaging/linux/package.sh` for Starbase: AppDir with both executables + dispatcher AppRun, pinned appimagetool/runtime, zsync update info for `DomeShifter/Starbase`, tar.gz; deb/rpm through nfpm optional | M | P1, D1a, D1b | Can use stub binaries first |
| P3 | Flatpak manifest (two cargo modules, two `cargo-sources.json`, git sources for both forks + fonts/word lists with sha256, trimmed finish-args), local `flatpak-builder` check | M | P1, D1a, D1b | Parallel with P2 |
| P4 | Update notice: check `api.github.com/repos/DomeShifter/Starbase/releases/latest`, show "new version" with a link; AppImage only (Flatpak: Flathub updates; deb/rpm: the package manager). Write our own small version rather than patching `craft-update` (which hardcodes `ai.storyteller.*` IDs). | S | D1a | |
| P5 | CI `desktop.yml` (our own, not craft-common-actions): `ubuntu-22.04` x86_64 + `ubuntu-22.04-arm`; build, package AppImage + `.flatpak` + tar, SHA256SUMS, GPG-sign, attach to a tagged release; Flatpak build check on PRs | M | P2, P3 | Needs a GPG release key as a repo secret (James) |
| Q1 | Tests: kittest for the local home and the connect dialog; AppImage smoke test in CI (run `--version`, open a sample xlsx headless under Xvfb, then quit) | S–M | D3, D6, P2 | |
| P6 | Flathub submission PR + reviewer back-and-forth | S work, weeks waiting | P3, first tagged release with metainfo screenshots | Last |

### Order of work

**Wave 1 (parallel):** D1a, D1b, S1, P1.

**Wave 2 (parallel):** D2a, D2b, D4, P2, P3, P4.

**Wave 3:** D3, D5 and D6 can run in parallel with each other once D2a's trait exists. P5 follows P2 and P3.

**Wave 4:** Q1, the first tagged desktop release, then P6.

Critical path: D1a → D2a → D6/D3 → Q1 → release. That is roughly 2–3 weeks of agent time if waves run in parallel.

### Phases 3–4 in brief (for later assignment)

**Windows (Phase 3)**

| Job | Size | Notes |
|---|---|---|
| W1 | M | WiX MSI from upstream's `.wxs` + install-policy (two executables, three shortcuts, associations, new UpgradeCode) |
| W2 | S | Portable zip |
| W3 | S | `windows-latest` + `windows-11-arm` CI jobs |
| W4 | S | winget manifest |
| W5 | M | SignPath Foundation application and integration. The wait is external. |

**Mac (Phase 4)**

| Job | Size | Notes |
|---|---|---|
| M1 | M | Universal .app, Info.plist UTIs, Apple Events file opens |
| M2 | S | DMG |
| M3 | M | Developer ID signing + notarytool in CI on `macos-15`. Needs James's Apple Developer account and a decision on whose name is on the certificate. |

### Decisions for James before work starts

1. Local mode scope: a Starbase folder inside Documents, or all of Documents?
2. A GPG release-signing key for Linux assets: generate a new one for the project?
3. Windows signing route: SignPath Foundation (no personal name on the certificate) or Artifact Signing (needs a UK company)?
4. Mac: Apple Developer Program ($99/yr, legal name on the certificate), or ship unsigned to testers only?
5. Linux package formats: deb/rpm too, or AppImage + Flatpak only? Upstream ships all of them; each extra format adds CI time and support.

### Sources

- Local: `stardocs/upstream/wordcraft/packaging/**`, `.github/workflows/release.yml`; `starsheets/Cargo.toml`, `stardocs/Cargo.toml` and both `Cargo.lock`; `starsheets/app/src/main.rs`; `stardocs/app/src/{server.rs,native_stubs.rs}`; `starsheets/crates/server/src/{main.rs,auth.rs,guard.rs,live.rs}`.
- GitHub:
  - `storytold/gridcraft@v0.19.1`: tree, `.desktop`, Flatpak manifest
  - Release asset lists for gridcraft v0.19.1 and wordcraft v0.18.1
  - `storytold/craft-common-actions` `release.yml@main`
  - `storytold/craft-libs@6e03f65b` `crates/update`
  - egui rev on `storytold/{gridcraft,wordcraft}` `main`
- Docs:
  - docs.flathub.org: for-app-authors/requirements and /linter
  - learn.microsoft.com: azure/artifact-signing overview, quickstart (prerequisites) and FAQ
  - signpath.org/terms
