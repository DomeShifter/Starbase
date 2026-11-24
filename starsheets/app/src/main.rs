//! StarSheets: a spreadsheet for the web that looks and works like Google Sheets, built on
//! GridCraft's engine and grid (see README.md and docs/architecture.md).
//!
//! Build with `trunk build --release` from `app/` after `scripts/fetch-upstream.sh`.

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
#[cfg(target_arch = "wasm32")]
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
mod merge;
mod messages;
mod namespanel;
mod notes;
mod panels;
#[cfg(target_arch = "wasm32")]
mod prefs;
mod protectpanel;
mod refs;
mod settings;
#[cfg(target_arch = "wasm32")]
mod share;
mod sheets_formulas;
mod shell;
mod spelling;
mod theme;
#[cfg(target_arch = "wasm32")]
mod testhook;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("starsheets only runs in the browser: build it with `trunk build --release` in app/");
}
