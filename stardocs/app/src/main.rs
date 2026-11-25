//! StarDocs: a Google Docs-style word processor in the browser and on the desktop, on
//! WordCraft's engine. Adapted from WordCraft's apps/wordcraft-web (MIT OR Apache-2.0); our window
//! is in `shell`, shared by both: the browser runs it through `web`, the desktop program
//! (`stardocs [FILE]`, local files) through `native`.
//!
//! Upstream notes, kept: WordCraft in the browser.
//!
//! Runs the same [`wordcraft_ui_egui::WordApp`] as the desktop app through eframe's web runner
//! (WebGL2 through glow; upstream uses wgpu). Build with `trunk build --release` from this
//! directory (output in `dist/web`).
//!
//! Differences from the desktop app: no TCP control channel; Open and Insert › Pictures use the
//! browser file picker (bytes arrive through `Services::inbox`); Save and Export download.
//! Preferences (theme, interface size, editing options…) are kept in `localStorage` (`prefs`).
//! URL flags: `?sample` opens the sample document.
//! `?host=parent` (inside an `<iframe>`) talks to a same-origin parent page instead: it sends the
//! document to open and stores what WordCraft saves (`host`). A page can also host WordCraft in
//! its own document (no iframe) with `data-host="page"` on the canvas, talking over DOM
//! `CustomEvent`s on the canvas. `&author=` names the user for comments and tracked changes in
//! either host mode.
// Windows release builds are windowed programs: no console window opens beside them.
// `--version`/`--help` join the terminal that started them (starbase_connect::attach_terminal).
#![cfg_attr(all(windows, not(debug_assertions), not(test)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// The window and what it needs: both the browser and the desktop.
mod bundled_fonts;
// A commenter's ids against the server's (server.rs); tested natively.
#[cfg(any(target_arch = "wasm32", test))]
mod comment_ids;
mod download;
mod icons;
mod looks;
mod mystyles;
mod prefs;
mod shell;
mod shortcuts;
mod theme;
mod versions;
mod wordcount;

#[cfg(any(target_arch = "wasm32", test))]
mod save_fallback;

// The browser.
#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod server;
#[cfg(target_arch = "wasm32")]
mod share;
#[cfg(target_arch = "wasm32")]
mod testhook;
#[cfg(target_arch = "wasm32")]
mod web;

// The desktop: the document is a local file (`localfile` stands where the browser's server link
// does), and there is no Share dialog without a server.
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(not(target_arch = "wasm32"))]
mod localfile;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
mod native_stubs;
#[cfg(not(target_arch = "wasm32"))]
mod serverfile;
#[cfg(not(target_arch = "wasm32"))]
mod updates;
#[cfg(not(target_arch = "wasm32"))]
use localfile as server;
#[cfg(not(target_arch = "wasm32"))]
use native_stubs::share;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod local_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod shell_tests;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::main()
}
