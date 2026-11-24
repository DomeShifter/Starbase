//! StarDocs: a Google Docs-style word processor in the browser, on WordCraft's engine.
//! Adapted from WordCraft's apps/wordcraft-web (MIT OR Apache-2.0); our window is in `shell`.
//!
//! Upstream notes, kept: WordCraft in the browser.
//!
//! Runs the same [`wordcraft_ui_egui::WordApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`).
//!
//! Differences from the desktop app: no TCP control channel; Open and Insert › Pictures use the
//! browser file picker (bytes arrive through `Services::inbox`); Save and Export download.
//! Preferences (theme, interface size, editing options…) are kept in `localStorage` (`prefs`).
//! URL flags: `?webgl` forces WebGL2; `?sample` opens the sample document.
//! `?host=parent` (inside an `<iframe>`) talks to a same-origin parent page instead: it sends the
//! document to open and stores what WordCraft saves (`host`). A page can also host WordCraft in
//! its own document (no iframe) with `data-host="page"` on the canvas, talking over DOM
//! `CustomEvent`s on the canvas. `&author=` names the user for comments and tracked changes in
//! either host mode.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_arch = "wasm32", test))]
mod prefs;
#[cfg(any(target_arch = "wasm32", test))]
mod save_fallback;

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod icons;
#[cfg(target_arch = "wasm32")]
mod server;
#[cfg(target_arch = "wasm32")]
mod share;
#[cfg(target_arch = "wasm32")]
mod shell;
#[cfg(target_arch = "wasm32")]
mod shortcuts;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod wordcount;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("stardocs only runs in the browser: build it with `trunk build --release` in stardocs/app");
}
