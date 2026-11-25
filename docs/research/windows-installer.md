# Windows installer: which kind, and why (Phase 3 look-ahead)

What Starbase needs from it:

- installs per user, without an administrator;
- Start menu entries for Starbase Drive, Sheets and Docs (and Slides once it has a desktop program);
- "Open with" for .xlsx/.ods/.csv/.docx/.odt (.pptx with Slides), never taking over the default app;
- uninstalling leaves the person's files and settings;
- a new version replaces the old one in place;
- SignPath can sign it later (it signs MSI, MSIX and EXE);
- a winget manifest can point at it later.

## The options

| | WiX MSI | Inno Setup EXE | MSIX |
|---|---|---|---|
| Per user, no admin | Yes (`Scope="perUser"`, installs to `%LOCALAPPDATA%\Programs`) | Yes (`PrivilegesRequired=lowest`) | Yes |
| Start menu entries | Yes | Yes | Yes (one per app in the manifest) |
| "Open with" without taking the default | Yes: registry `OpenWithProgids` + Default apps "Capabilities" | Yes: the same registry keys, written by script | Yes: `FileTypeAssociation` in the manifest; Windows never makes it the default by itself |
| Uninstall keeps user files | Yes: MSI removes only what it installed | Yes | Yes, but the app's own `AppData` inside the package is removed with it |
| Upgrade in place | Yes: `MajorUpgrade` by UpgradeCode | Yes: by `AppId` | Yes |
| **Installs unsigned** | **Yes** (SmartScreen warns on download) | **Yes** (same) | **No**: Windows installs only signed MSIX, or with Developer Mode / a trusted test certificate |
| Signing later (SignPath) | Sign the two .exe, then the .msi | Sign the setup EXE; its uninstaller is built during compilation and has to be signed then, through Inno's `SignTool` hook, which doesn't fit SignPath's "sign the finished file" flow well | Sign the .msix |
| winget | `InstallerType: wix`, ProductCode known, scope `user` | `InstallerType: inno` | `msix` |
| Company IT (Intune, Group Policy) | Native: MSI is what they deploy | Needs switches per installer | Native on Intune |
| Our code to reuse | GridCraft's `packaging/windows/gridcraft.wxs` (WiX v4 schema, tested associations) | none | none |
| Tooling cost | WiX v7: free for us (its Open Source Maintenance Fee applies to organisations with over $10,000 a year in revenue); CI accepts its EULA with `-acceptEula wix7` | Free | Free (`makeappx`, in the Windows SDK) |

## Pick: WiX MSI

- MSIX is out while Starbase is unsigned: it won't install at all.
- Between MSI and Inno, MSI wins on the parts that matter later:
  - SignPath signs a finished MSI cleanly; Inno's built-in uninstaller doesn't fit that flow.
  - IT departments deploy MSI as it is.
  - winget gets a ProductCode.
  - GridCraft's tested `.wxs` gives us the association pattern ("Open with" + Default apps, never the default).
- What we give up: a fancy setup wizard. The MSI has no dialogs of its own yet, so a double-click shows Windows' small progress window and installs straight away for the current user. A short welcome/licence dialog (WiX's UI extension) can come with Phase 3 proper.

**Decision for James:** CI uses WiX v7 and accepts its EULA (`-acceptEula wix7`). Starbase has no revenue, so the WiX Open Source Maintenance Fee doesn't apply. If that ever changes, the fee applies, or we pin WiX v5, which needs no EULA but has no community support any more.

## What is built (packaging/windows/)

| File | What it does |
|---|---|
| `Starbase.wxs` | The package. Scope per user; both programs in `%LOCALAPPDATA%\Programs\Starbase`. It adds three Start menu entries: Drive (`starsheets --home`), Sheets and Docs. File types: ProgIds `Starbase.Sheets.Workbook` (.xlsx .xlsm .xlsb .ods), `Starbase.Sheets.TextData` (.csv .tsv) and `Starbase.Docs.Document` (.docx .docm .dotx .dotm .doc .odt .rtf). They go under `OpenWithProgids` and the Default apps "Capabilities" only. The package also adds App Paths (so `starsheets`/`stardocs` work from Run) and the licences. Its UpgradeCode never changes. |
| `make-ico.py` | Packs the PNGs from `packaging/linux/icons.sh` into .ico files (16–256 px). Nothing generated is committed. |
| `build-msi.ps1` | Installs WiX 7.0.1 (a .NET tool) if missing, then builds the MSI. |
| `test-msi.ps1` | Install → check files, Start menu entries, "Open with", Default apps, and that the defaults are unchanged → start Drive and Docs from their Start menu entries and Sheets with a CSV through the registered command → upgrade in place with a newer build (still one entry in Installed apps) → uninstall (everything gone, the person's files in Documents\Starbase and %APPDATA%\Starbase kept). |

CI: `desktop-other.yml` jobs `windows-icons` (Linux: render + pack) and `windows-installer`
(windows-latest: build, test, upload `starbase-<version>-x64-setup` for 7 days). Versions:
a `vX.Y.Z` tag gives X.Y.Z; other builds give `<last tag>-dev.<commits since>`, with MSI
version `<last tag>.<commits since>`.

## Left for Phase 3 proper

- **Signing.** SignPath signs `starsheets.exe` and `stardocs.exe` before `wix build`, then the MSI. Until then SmartScreen warns ("Windows protected your PC"; "More info" › "Run anyway").
- **Icons inside the .exe files.** Taskbar and Explorer show a generic icon for the files themselves. The shortcuts and file types have ours. This needs a resource step in each app's build (`winres`/`embed-resource`).
- **A welcome dialog**, an optional desktop shortcut (GridCraft's dialog does both), and the "launch now" checkbox.
- **Slides.** Add its program, Start menu entry and `.pptx`/`.odp` ProgIds once it has a desktop build.
- **winget.** Write the manifest (`wingetcreate new <release MSI URL>`) after the first release with an MSI attached. Scope `user`, InstallerType `wix`.
- **Per-machine installs** (all users, admin). GridCraft's `perUserOrMachine` + install policy is the pattern, if IT departments ask.
- **Arm64.** A `windows-11-arm` build and a second MSI.
