//! Fonts we bundle as stand-ins for the fonts most documents use, on the web and on desktops
//! without them: Arimo for Arial (a new document's default, as in Google Docs), Carlito for
//! Calibri and Caladea for Cambria (Word's defaults since 2007). Each is drawn to its original's
//! widths, and WordCraft already draws a missing Arial, Calibri or Cambria in its installed metric
//! twin, so with these present the text keeps the original's widths and breaks lines where Word
//! and Google Docs do, with no warning in the font box. They are added as a document's fonts are:
//! for this run, not as user fonts.

/// Arimo's four faces (app/fonts, Apache License 2.0), in the program: every new document uses it.
const ARIMO: [&[u8]; 4] = [
    include_bytes!("../fonts/Arimo-Regular.ttf"),
    include_bytes!("../fonts/Arimo-Bold.ttf"),
    include_bytes!("../fonts/Arimo-Italic.ttf"),
    include_bytes!("../fonts/Arimo-BoldItalic.ttf"),
];

/// Carlito's and Caladea's faces (app/fonts/office, SIL Open Font License 1.1): file, family.
pub const OFFICE: [(&str, &str); 8] = [
    ("Carlito-Regular.ttf", "Carlito"),
    ("Carlito-Bold.ttf", "Carlito"),
    ("Carlito-Italic.ttf", "Carlito"),
    ("Carlito-BoldItalic.ttf", "Carlito"),
    ("Caladea-Regular.ttf", "Caladea"),
    ("Caladea-Bold.ttf", "Caladea"),
    ("Caladea-Italic.ttf", "Caladea"),
    ("Caladea-BoldItalic.ttf", "Caladea"),
];

/// The desktop program carries Carlito and Caladea inside it, in [`OFFICE`]'s order.
#[cfg(not(target_arch = "wasm32"))]
const OFFICE_BYTES: [&[u8]; 8] = [
    include_bytes!("../fonts/office/Carlito-Regular.ttf"),
    include_bytes!("../fonts/office/Carlito-Bold.ttf"),
    include_bytes!("../fonts/office/Carlito-Italic.ttf"),
    include_bytes!("../fonts/office/Carlito-BoldItalic.ttf"),
    include_bytes!("../fonts/office/Caladea-Regular.ttf"),
    include_bytes!("../fonts/office/Caladea-Bold.ttf"),
    include_bytes!("../fonts/office/Caladea-Italic.ttf"),
    include_bytes!("../fonts/office/Caladea-BoldItalic.ttf"),
];

/// Adds `files` (bytes, family) to the font database; `what` names them in a warning.
fn add_files(files: Vec<(Vec<u8>, String)>, what: &str) {
    let n = files.len();
    if wordcraft_fonts::FontDb::global().add_document_fonts(files) != n {
        log::warn!("{what} didn't load fully");
    }
}

/// Adds the bundled fonts before any text is laid out. In the browser Carlito and Caladea are
/// not in the program (3.0 MB, 0.83 MB brotli: a tenth more to download): [`fetch_office`] fetches them.
pub fn add() {
    add_files(ARIMO.iter().map(|b| (b.to_vec(), "Arimo".to_string())).collect(), "Arimo (the bundled stand-in for Arial)");
    #[cfg(not(target_arch = "wasm32"))]
    add_files(
        OFFICE_BYTES.iter().zip(OFFICE).map(|(b, (_, family))| (b.to_vec(), family.to_string())).collect(),
        "Carlito and Caladea (the bundled stand-ins for Calibri and Cambria)",
    );
}

/// The browser: fetches Carlito and Caladea from beside the page (`office/`, copied there by
/// Trunk) while the document opens, and adds them once all have arrived. A document already
/// drawn in a fallback font is laid out again in them. A file that fails is logged and left out.
#[cfg(target_arch = "wasm32")]
pub fn fetch_office(ctx: egui::Context) {
    use std::cell::RefCell;
    use std::rc::Rc;
    let got: Rc<RefCell<(usize, Vec<(Vec<u8>, String)>)>> = Rc::default();
    for (file, family) in OFFICE {
        let (got, ctx) = (got.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("office/{file}");
            match crate::server::fetch("GET", &url, None, None).await {
                Ok((200, _, bytes)) => got.borrow_mut().1.push((bytes, family.to_string())),
                Ok((code, ..)) => log::warn!("{url}: error {code}"),
                Err(e) => log::warn!("{url}: {e}"),
            }
            let mut g = got.borrow_mut();
            g.0 += 1;
            if g.0 == OFFICE.len() && !g.1.is_empty() {
                add_files(std::mem::take(&mut g.1), "Carlito and Caladea (the bundled stand-ins for Calibri and Cambria)");
                ctx.request_repaint();
            }
        });
    }
}
