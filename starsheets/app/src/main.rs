//! StarSheets: a spreadsheet for the web that looks and works like Google Sheets, built on
//! GridCraft's engine and grid (see README.md and docs/architecture.md).
//!
//! Build the web app with `trunk build --release` from `app/` after `scripts/fetch-upstream.sh`.
//! `cargo build -p starsheets` builds the desktop program instead (native.rs).

#[cfg(target_arch = "wasm32")]
mod account;
mod autolink;
mod bottom;
mod calcpreview;
mod cfpanel;
mod chartpanel;
mod colourpicker;
#[cfg(target_arch = "wasm32")]
mod copyto;
mod currency;
mod customdate;
mod customnumber;
mod datepicker;
mod dvpanel;
mod editmenu;
mod errortip;
#[cfg(target_arch = "wasm32")]
mod filebrowser;
#[cfg(target_arch = "wasm32")]
mod files;
mod findreplace;
mod fonts;
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
mod spelling;
mod store;
#[cfg(target_arch = "wasm32")]
mod testhook;
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
