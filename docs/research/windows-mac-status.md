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
| Starts and stays up 15 s on the runner | Sheets: yes, on wgpu's DX12 software renderer. Docs: no, exits at once ("egui_glow requires opengl 2.0+"; see "OpenGL" below) | both: yes |
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

## Gaps for Phase 3 (Windows) and Phase 4 (macOS)

| Area | What happens now | What's needed |
|---|---|---|
| Installer | None. Windows: a portable zip; macOS: a bare executable (no `.app`, no Dock icon, no Info.plist). | WiX MSI (3c), `.app` + DMG (3d) |
| Signing | Unsigned. Windows SmartScreen: "Windows protected your PC", publisher Unknown, "More info" › "Run anyway"; every release starts with no reputation. macOS Gatekeeper blocks it until "Open Anyway" in Privacy & Security. | SignPath (Windows), Developer ID + notarisation (macOS) |
| Console window | No `windows_subsystem = "windows"`: on Windows each program opens a black console window beside its own. | One `cfg_attr` line in each `main.rs` (then `--version` prints only when redirected) |
| OpenGL (Docs) | Starbase Docs draws only with OpenGL 3.3 (glow). On a computer with only Microsoft's basic driver (VMs, remote desktop, no GPU driver) it gets GL 1.1 and can't start. Sheets falls back to wgpu (DX12 software) and starts. | wgpu as Docs' native renderer, or a GL-less fallback |
| File associations | None (double-clicking a .xlsx/.docx doesn't offer Starbase). macOS also needs Apple Events for files opened from Finder (upstream's `apple_events.rs`). | In the MSI / Info.plist |
| Keyring | keyring 4.2 picks Credential Manager / Keychain by itself on first use; nothing to set up. Not exercised by CI (tests use the file fallback). Unsigned macOS builds may ask for Keychain access again after every update. On Windows the fallback `token` file isn't restricted to the user (0600 is Unix-only); `%APPDATA%` is per-user, so low risk. | A manual sign-in test on each system |
| Paths | Hand-rolled from environment variables, no `dirs` crate. Sheets prefs: `%APPDATA%\starbase\sheets` / macOS `~/.config/starbase/sheets`; Sheets data: `%APPDATA%\starbase` / macOS `~/.local/share/starbase`; server-file cache: `HOME/.cache` (temp folder on Windows) / macOS `~/.cache`; Docs prefs need `HOME`, so on Windows Docs' preferences aren't saved. Documents = `%USERPROFILE%\Documents`: wrong when Windows moved it (OneDrive "Documents backup", a relocated folder). | One shared helper (e.g. the `dirs` crate: `document_dir`, `config_dir`, `data_dir`, `cache_dir`) used by both programs and connect |
| Starting other programs | `start_detached` puts children in their own process group on Unix only; on Windows a Docs window started from Drive shares its console. | `CREATE_NEW_PROCESS_GROUP`/`DETACHED_PROCESS` on Windows |
| User's name | Windows: the login name (`USERNAME`), not the full name. | Optional: `GetUserNameExW` |
| Fonts | egui's bundled fonts and Docs' bundled fonts work everywhere; no CJK/emoji fallback from the system on Windows/macOS was checked. | A look at CJK text on both |
| IME | Not tested. winit (our fork) supports Windows IME and macOS input methods; GridCraft's and WordCraft's text fields haven't been tried with them. | A manual test (Japanese/Chinese input) |
| Universal macOS build | arm64 only. | x86_64 + `lipo` (3d) |
