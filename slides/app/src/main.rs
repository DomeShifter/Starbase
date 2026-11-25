//! Starbase Slides: a Google Slides-style presentation editor in the browser and on the desktop,
//! on DeckCraft's engine. Adapted from DeckCraft's apps/deckcraft-web (MIT OR Apache-2.0): our
//! window is in `shell` (menus and toolbar; DeckCraft's own title bar, ribbon and status bar are
//! off), the browser runs it through `web`, the desktop program (`starslides [FILE]`, local
//! .pptx files, or a connected server's) through `native`.
//!
//! - Opened from the Starbase home screen (`?file=<id>`, or `?new=1&folder=<folder>`), the
//!   presentation is read from and saved to the Starbase server as a .pptx (`server`).
//! - `?host=parent` (inside a same-origin `<iframe>`) or `data-host="page"` on the canvas: the
//!   page keeps the files instead (`host`, DeckCraft's bridge), `&author=` names the user.
//! - Otherwise a new presentation (`?sample`: DeckCraft's sample deck); saves download.
//! - `?test` puts the test hook `window.starslides` on the page (`testhook`); DeckCraft's own
//!   `window.deckcraft` page API is always there (`api`).
//! - The presenter view opens as a same-origin popup window (`presenter`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// The desktop program on Windows: no console window beside it (as Sheets and Docs);
// `--version`/`--help` join the terminal that started them (starbase_connect::attach_terminal).
#![cfg_attr(all(windows, not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod fonts;
mod icons;
mod prefs;
mod shell;
mod theme;

#[cfg(any(target_arch = "wasm32", test))]
mod api;
#[cfg(target_arch = "wasm32")]
mod fullscreen;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod presenter;
#[cfg(target_arch = "wasm32")]
mod server;
#[cfg(target_arch = "wasm32")]
mod testhook;
#[cfg(target_arch = "wasm32")]
mod web;

// The desktop: the presentation is a local file (`localfile`), or one on a Starbase server this
// computer is connected to (`connect`, `serverfile`).
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(not(target_arch = "wasm32"))]
mod localfile;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
mod serverfile;

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
