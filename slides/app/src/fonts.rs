//! Fonts we bundle, so slides look the same on every computer (a browser can't use the fonts
//! installed on it). The same idea as StarDocs' `bundled_fonts.rs`, for the fonts decks use:
//!
//! - Inter Regular and Bold, in the program: DeckCraft's default theme font (a new presentation's
//!   text), the faces DeckCraft's own web build carries.
//! - Fetched from `more/` beside the page after start (`slides/app/fonts/more`, copied there by
//!   Trunk), so they don't add to the first download: Inter Italic; Liberation Sans for Arial
//!   (the same design and widths as StarDocs' Arimo, and the name DeckCraft already draws a
//!   missing Arial in); Carlito for Calibri and Caladea for Cambria (PowerPoint's defaults from
//!   2007 to 2023), as StarDocs. Each is drawn to its original's widths, so text breaks where it
//!   does in PowerPoint. Once they arrive the slides are drawn again.
//!
//! All SIL Open Font License 1.1 (licences beside the files).

/// Inter Regular and Bold, in the program.
const INTER: [&[u8]; 2] = [include_bytes!("../fonts/Inter-Regular.ttf"), include_bytes!("../fonts/Inter-Bold.ttf")];

/// The faces fetched after start, from `more/` (the desktop program has them in it: [`MORE_BYTES`]).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code, reason = "the browser's list; the tests check it"))]
pub const MORE: [&str; 13] = [
    "Inter-Italic.ttf",
    "LiberationSans-Regular.ttf",
    "LiberationSans-Bold.ttf",
    "LiberationSans-Italic.ttf",
    "LiberationSans-BoldItalic.ttf",
    "Carlito-Regular.ttf",
    "Carlito-Bold.ttf",
    "Carlito-Italic.ttf",
    "Carlito-BoldItalic.ttf",
    "Caladea-Regular.ttf",
    "Caladea-Bold.ttf",
    "Caladea-Italic.ttf",
    "Caladea-BoldItalic.ttf",
];

/// Adds the fonts in the program, before any slide is drawn.
pub fn add() {
    let db = deckcraft_fonts::FontDb::global();
    let added: usize = INTER.iter().map(|b| db.add_font(b.to_vec())).sum();
    if added < INTER.len() {
        log::warn!("Inter (the default slide font) didn't load fully");
    }
    // The desktop program carries the rest in it too: nothing to fetch.
    #[cfg(not(target_arch = "wasm32"))]
    for bytes in MORE_BYTES {
        db.add_font(bytes.to_vec());
    }
}

/// [`MORE`]'s files, in the desktop program.
#[cfg(not(target_arch = "wasm32"))]
const MORE_BYTES: [&[u8]; 13] = [
    include_bytes!("../fonts/more/Inter-Italic.ttf"),
    include_bytes!("../fonts/more/LiberationSans-Regular.ttf"),
    include_bytes!("../fonts/more/LiberationSans-Bold.ttf"),
    include_bytes!("../fonts/more/LiberationSans-Italic.ttf"),
    include_bytes!("../fonts/more/LiberationSans-BoldItalic.ttf"),
    include_bytes!("../fonts/more/Carlito-Regular.ttf"),
    include_bytes!("../fonts/more/Carlito-Bold.ttf"),
    include_bytes!("../fonts/more/Carlito-Italic.ttf"),
    include_bytes!("../fonts/more/Carlito-BoldItalic.ttf"),
    include_bytes!("../fonts/more/Caladea-Regular.ttf"),
    include_bytes!("../fonts/more/Caladea-Bold.ttf"),
    include_bytes!("../fonts/more/Caladea-Italic.ttf"),
    include_bytes!("../fonts/more/Caladea-BoldItalic.ttf"),
];

/// The families DeckCraft can draw with now (the test hook reports them).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code, reason = "the browser's test hook; the tests check it"))]
pub fn families() -> Vec<String> {
    deckcraft_fonts::FontDb::global().families()
}

/// The browser: fetches [`MORE`] and adds each face as it arrives; `arrived` is told how many
/// were added, so the slides can be drawn again. A file that fails is logged and left out.
#[cfg(target_arch = "wasm32")]
pub fn fetch_more(arrived: std::rc::Rc<std::cell::Cell<usize>>, ctx: egui::Context) {
    for file in MORE {
        let (arrived, ctx) = (arrived.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("more/{file}");
            match crate::server::fetch("GET", &url, None, None).await {
                Ok((200, _, bytes)) => {
                    let n = deckcraft_fonts::FontDb::global().add_font(bytes);
                    arrived.set(arrived.get() + n);
                    ctx.request_repaint();
                }
                Ok((code, ..)) => log::warn!("{url}: error {code}"),
                Err(e) => log::warn!("{url}: {e}"),
            }
        });
    }
}
