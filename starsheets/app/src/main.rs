//! StarSheets: a spreadsheet for the web that looks and works like Google Sheets, built on
//! GridCraft's engine and grid (see README.md and docs/architecture.md).
//!
//! Build the web app with `trunk build --release` from `app/` after `scripts/fetch-upstream.sh`.
//! `cargo build -p starsheets` builds the desktop program instead (native.rs).
// Windows release builds are windowed programs: no console window opens beside them.
// `--version`/`--help` join the terminal that started them (starbase_connect::attach_terminal).
#![cfg_attr(all(windows, not(debug_assertions), not(test)), windows_subsystem = "windows")]
#![recursion_limit = "256"]

#[cfg(target_arch = "wasm32")]
mod account;
// The leave-page guard: its rule is tested natively.
mod altcolours;
mod autolink;
mod bottom;
mod calcpreview;
mod cfpanel;
mod chartpanel;
mod colourpicker;
#[cfg(any(target_arch = "wasm32", test))]
mod unload_guard;
// The web app's "continue to" after sign-in; its check is also tested natively.
#[cfg(any(target_arch = "wasm32", test))]
mod continueto;
#[cfg(target_arch = "wasm32")]
mod copyto;
mod currency;
mod customdate;
mod customnumber;
mod datepicker;
mod dvpanel;
mod editmenu;
mod emacs_keys;
mod errortip;
mod external;
#[cfg(target_arch = "wasm32")]
mod filebrowser;
#[cfg(target_arch = "wasm32")]
mod files;
mod findreplace;
mod fonts;
mod formulaimages;
mod home;
mod hostdata;
mod house;
mod icons;
#[cfg(target_arch = "wasm32")]
mod images;
mod input;
mod linkcard;
#[cfg(target_arch = "wasm32")]
mod live;
#[cfg(not(target_arch = "wasm32"))]
mod localstore;
mod menuactions;
mod merge;
mod messages;
mod namespanel;
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod notes;
mod panels;
mod pivotpanel;
#[cfg(target_arch = "wasm32")]
mod prefs;
mod protectpanel;
mod refs;
#[cfg(not(target_arch = "wasm32"))]
mod remote;
mod settings;
#[cfg(target_arch = "wasm32")]
mod share;
mod sheets_formulas;
mod shell;
mod shortcuts;
mod smartfill;
mod spelling;
mod store;
#[cfg(target_arch = "wasm32")]
mod testhook;
#[cfg(target_arch = "wasm32")]
mod textmenu;
mod theme;
#[cfg(not(target_arch = "wasm32"))]
mod updates;
// File › Version history: the web app's (files keep their versions on the server); also built
// for its tests.
#[cfg(any(target_arch = "wasm32", test))]
mod versions;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    native::main()
}
