# Windows and macOS: where the desktop programs stand (look-ahead for Phases 3 and 4)

Checked by `.github/workflows/desktop-other.yml` on GitHub's `windows-latest` (x86_64) and
`macos-latest` (arm64) runners. Nothing is installed or signed there: that comes with Phases 3–4
(desktop-plan.md 3c, 3d).

## What builds and passes

| | Windows x86_64 | macOS arm64 |
|---|---|---|
| starsheets (Drive + Sheets): clippy, release build | yes | yes |
| starsheets + starbase-connect + starsheets-sync tests (107 + 3 + 20) | pass | pass |
| stardocs (Docs): clippy, release build | yes | yes |
| stardocs tests, egui_kittest window tests included (68) | pass (after the test fix below) | pass |
| Starts and stays up 15 s on the runner | Sheets: yes, on wgpu's DX12 software renderer. Docs: yes since windows-polish: "egui_glow requires opengl 2.0+", then "trying wgpu", then the same DX12 software renderer (runner log) | both: yes |
| Artifacts (unsigned, 7 days) | `*-windows-x86_64`, plus `starbase-windows-x64-portable.zip` | `*-macos-arm64` (bare executables, no .app) |

The egui_kittest tests run headless on both: they use egui's accessibility tree, no GPU and no
window.

## Fixed on the way (small, platform-only)

- `starsheets/crates/connect`: settings folder is now `%APPDATA%\Starbase` on Windows and
  `~/Library/Application Support/Starbase` on macOS (was `~/.config/starbase` on macOS).
- `starsheets/app/src/localstore.rs`: `passwd_name` is Unix-only (dead code on Windows broke
  clippy); the zombie-reaping test reads `/proc`, so it runs on Linux only.
- `stardocs/app/src/localfile.rs`: the home folder falls back to `USERPROFILE` (Windows has no
  `HOME`, so Docs had no Documents folder at all there).
- `stardocs/app/src/local_tests.rs`: the command-line test used `/tmp/…`, which isn't absolute on
  Windows.
- Windows builds link the C runtime statically (`+crt-static`): no Visual C++ Redistributable
  needed. The zip job checks each .exe needs only Windows' own DLLs.

## Windows polish (branch windows-polish)

| What | Now |
|---|---|
| Console window | Release builds on Windows are windowed programs (`windows_subsystem = "windows"` in both `main.rs`): no black console beside the window. `--version`, `--help` and command-line errors join the terminal that started them (`starbase_connect::attach_terminal`: `AttachConsole` + `CONOUT$`), only when output isn't already piped or redirected. In `cmd.exe` the text appears after the prompt has come back, as with any windowed program; `stardocs --version | more` or `> file` always works. Debug builds keep their console. |
| Docs without OpenGL 2.0+ | Docs tries OpenGL (glow) first, as before; when that fails (OpenGL, glutin or no-config errors) it starts again with wgpu, which uses WARP (software DX12) on Windows, as Sheets does the other way round. The Docs executable grows by wgpu's size. |
| Docs preferences | Kept in Starbase's settings folder (`starbase_connect::config_dir`): `%APPDATA%\Starbase\docs-ui.json` (before: not kept at all, as Windows has no `HOME`). Linux path unchanged. Sheets' preferences moved to the same folder (`…\Starbase\sheets`; Linux unchanged, macOS was `~/.config`). |
| Documents folder | Windows: the Documents Known Folder (`dirs::document_dir`, `SHGetKnownFolderPath`), which follows OneDrive's "Documents backup" or a move to another drive; macOS `~/Documents`; Linux unchanged (XDG). Both programs (`starbase_connect::system_documents_dir`). A Windows test on CI runners moves the folder (the `Personal` value under User Shell Folders) and checks it's followed. |
| Data (Recent list etc.) | Windows and macOS: Starbase's settings folder for both programs (Docs used `%USERPROFILE%\.local\share`, Sheets `~/.local/share` on macOS). |

Workflow changes for ci-speed (not made here): make the "Starts and stays up" step in
desktop-other.yml strict (drop `continue-on-error`) now that Docs starts on the runner without
OpenGL, and grep its log for `trying wgpu` on Windows to prove the fallback ran.

## Gaps for Phase 3 (Windows) and Phase 4 (macOS)

| Area | What happens now | What's needed |
|---|---|---|
| Installer | None. Windows: a portable zip; macOS: a bare executable (no `.app`, no Dock icon, no Info.plist). | WiX MSI (3c), `.app` + DMG (3d) |
| Signing | Unsigned. Windows SmartScreen: "Windows protected your PC", publisher Unknown, "More info" › "Run anyway"; every release starts with no reputation. macOS Gatekeeper blocks it until "Open Anyway" in Privacy & Security. | SignPath (Windows), Developer ID + notarisation (macOS) |
| File associations | None (double-clicking a .xlsx/.docx doesn't offer Starbase). macOS also needs Apple Events for files opened from Finder (upstream's `apple_events.rs`). | In the MSI / Info.plist |
| Keyring | keyring 4.2 picks Credential Manager / Keychain by itself on first use; nothing to set up. Not exercised by CI (tests use the file fallback). Unsigned macOS builds may ask for Keychain access again after every update. On Windows the fallback `token` file isn't restricted to the user (0600 is Unix-only); `%APPDATA%` is per-user, so low risk. | A manual sign-in test on each system |
| Paths | Settings, data and Documents fixed (above). Left: the server-file cache is `HOME/.cache` (the temp folder on Windows, `~/.cache` on macOS); eframe keeps window sizes in its own per-app folder. | `dirs::cache_dir` for the cache |
| Starting other programs | `start_detached` puts children in their own process group on Unix only; on Windows a Docs window started from Drive shares its console. | `CREATE_NEW_PROCESS_GROUP`/`DETACHED_PROCESS` on Windows |
| User's name | Windows: the login name (`USERNAME`), not the full name. | Optional: `GetUserNameExW` |
| Fonts | egui's bundled fonts and Docs' bundled fonts work everywhere; no CJK/emoji fallback from the system on Windows/macOS was checked. | A look at CJK text on both |
| IME | Not tested. winit (our fork) supports Windows IME and macOS input methods; GridCraft's and WordCraft's text fields haven't been tried with them. | A manual test (Japanese/Chinese input) |
| Universal macOS build | arm64 only. | x86_64 + `lipo` (3d) |
