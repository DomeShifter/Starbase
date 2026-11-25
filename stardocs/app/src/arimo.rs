//! Arimo, bundled: the stand-in for Arial on the web and on desktops without Arial.

/// Arimo's four faces (app/fonts, Apache License 2.0): drawn to Arial's widths.
const ARIMO: [&[u8]; 4] = [
    include_bytes!("../fonts/Arimo-Regular.ttf"),
    include_bytes!("../fonts/Arimo-Bold.ttf"),
    include_bytes!("../fonts/Arimo-Italic.ttf"),
    include_bytes!("../fonts/Arimo-BoldItalic.ttf"),
];

/// Add Arimo to the font database before any text is laid out. A browser (and many Linux
/// desktops) has no Arial, the default font of a new document (as in Google Docs); WordCraft
/// draws a missing Arial in its first installed metric twin (Liberation Sans, Helvetica, Arimo),
/// so with Arimo present Arial text keeps Arial's widths and breaks lines where Word and Google
/// Docs do, with no warning in the font box. Added as a document's fonts are: for this run, not
/// as a user font.
pub fn add() {
    let files = ARIMO.iter().map(|b| (b.to_vec(), "Arimo".to_string())).collect();
    if wordcraft_fonts::FontDb::global().add_document_fonts(files) != ARIMO.len() {
        log::warn!("Arimo (the bundled stand-in for Arial) didn't load fully");
    }
}
