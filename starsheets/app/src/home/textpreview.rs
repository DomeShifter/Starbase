//! Text files in Drive's preview: a minimal version (plain monospace text) until the full one
//! (highlighting, Markdown) lands.

/// Shows `bytes` (a text file called `name`) as plain monospace text, scrolling.
pub fn show(ui: &mut egui::Ui, name: &str, bytes: &[u8]) {
    let _ = name;
    let text = String::from_utf8_lossy(bytes);
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(text.as_ref()).monospace().color(egui::Color32::from_gray(30))).extend());
    });
}
