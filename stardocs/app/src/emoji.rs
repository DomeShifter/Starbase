//! Insert › Emoji: a small picker (search box + grid) that types the chosen emoji at the caret,
//! as Docs' emoji picker does. The emoji go in as ordinary text (`insert.symbol`).

use egui::{Align, Layout, RichText, vec2};
use serde_json::json;
use wordcraft_ui_egui::WordApp;

use crate::shell::{TEXT, TEXT_DIM, run};

/// (emoji, words it is found by).
const LIST: &[(&str, &str)] = &[
    ("😀", "grinning smile happy"),
    ("😂", "joy laugh tears"),
    ("🙂", "slight smile"),
    ("😉", "wink"),
    ("😊", "blush smile"),
    ("😍", "heart eyes love"),
    ("😘", "kiss"),
    ("😎", "cool sunglasses"),
    ("🤔", "thinking"),
    ("😐", "neutral"),
    ("😢", "cry sad"),
    ("😭", "sob"),
    ("😡", "angry"),
    ("😱", "scream shock"),
    ("😴", "sleep"),
    ("🥳", "party celebrate"),
    ("🤗", "hug"),
    ("🙄", "eye roll"),
    ("👍", "thumbs up yes good"),
    ("👎", "thumbs down no bad"),
    ("👏", "clap applause"),
    ("🙏", "pray thanks please"),
    ("💪", "strong muscle"),
    ("👋", "wave hello"),
    ("✌️", "peace victory"),
    ("🤝", "handshake deal"),
    ("👀", "eyes look"),
    ("❤️", "heart love red"),
    ("💔", "broken heart"),
    ("💙", "blue heart"),
    ("💚", "green heart"),
    ("⭐", "star"),
    ("✨", "sparkles"),
    ("🔥", "fire hot"),
    ("💡", "idea light bulb"),
    ("🎉", "party popper tada"),
    ("🎂", "birthday cake"),
    ("🎁", "gift present"),
    ("🏆", "trophy win"),
    ("✅", "check done tick"),
    ("❌", "cross wrong no"),
    ("⚠️", "warning"),
    ("❓", "question"),
    ("❗", "exclamation"),
    ("📌", "pin"),
    ("📎", "paperclip attach"),
    ("📅", "calendar date"),
    ("⏰", "alarm clock time"),
    ("📧", "email mail"),
    ("📞", "phone call"),
    ("💻", "laptop computer"),
    ("📱", "mobile phone"),
    ("📷", "camera photo"),
    ("🎵", "music note"),
    ("📚", "books study"),
    ("📝", "memo note write"),
    ("🔒", "lock secure"),
    ("🔑", "key"),
    ("💰", "money bag"),
    ("🚀", "rocket launch"),
    ("🚗", "car"),
    ("✈️", "plane travel"),
    ("🏠", "home house"),
    ("🌍", "earth world globe"),
    ("☀️", "sun sunny"),
    ("🌙", "moon night"),
    ("☁️", "cloud"),
    ("🌧️", "rain"),
    ("❄️", "snow cold"),
    ("🌈", "rainbow"),
    ("🌱", "seedling plant grow"),
    ("🌸", "flower blossom"),
    ("🍕", "pizza food"),
    ("🍔", "burger food"),
    ("☕", "coffee tea hot drink"),
    ("🍺", "beer drink"),
    ("🍎", "apple fruit"),
    ("🐶", "dog"),
    ("🐱", "cat"),
    ("🐻", "bear"),
    ("🦊", "fox"),
    ("🐢", "turtle"),
];

#[derive(Default)]
pub struct Emoji {
    pub open: bool,
    filter: String,
    /// The picker was drawn last frame, so opening it clears and focuses the search box.
    shown: bool,
}

/// The emoji whose words contain every word of `filter` (all of them when it is empty).
pub fn matching(filter: &str) -> Vec<&'static str> {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    LIST.iter().filter(|(_, names)| words.iter().all(|w| names.contains(w.as_str()))).map(|(e, _)| *e).collect()
}

impl Emoji {
    pub fn ui(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        if !self.open {
            self.shown = false;
            return;
        }
        let opening = !self.shown;
        self.shown = true;
        if opening {
            self.filter.clear();
        }
        let mut close = false;
        let mut pick = None;
        let modal = egui::Modal::new(egui::Id::unique("stardocs_emoji")).frame(crate::looks::dialog_frame());
        let shown = modal.show(ctx, |ui| {
            ui.set_width(360.0);
            ui.horizontal(|ui| {
                crate::looks::title(ui, "Emoji");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            let search = ui
                .scope(|ui| {
                    crate::looks::field_style(ui);
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search emoji").desired_width(f32::INFINITY).text_color(TEXT))
                })
                .inner;
            if opening {
                search.request_focus();
            }
            ui.add_space(8.0);
            // Only emoji the interface fonts can draw: egui refuses a missing glyph.
            let font = egui::FontId::proportional(22.0);
            let found: Vec<&str> = matching(&self.filter).into_iter().filter(|e| ui.fonts_mut(|f| f.has_glyphs(&font, e))).collect();
            if found.is_empty() {
                ui.label(RichText::new("No emoji found").color(TEXT_DIM));
            }
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for e in found {
                        let r = ui.add(egui::Button::new(RichText::new(e).size(22.0)).frame(false).min_size(vec2(36.0, 36.0)));
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, e));
                        if r.clicked() {
                            pick = Some(e);
                        }
                    }
                });
            });
        });
        if shown.should_close() {
            close = true;
        }
        if let Some(e) = pick {
            self.open = false;
            run(app, "insert.symbol", json!({ "char": e }));
        } else if close {
            self.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matching;

    #[test]
    fn search_finds_by_every_word() {
        assert_eq!(matching("").len(), super::LIST.len());
        assert!(matching("thumbs up").contains(&"👍"));
        assert!(!matching("thumbs up").contains(&"👎"));
        assert!(matching("zzzz").is_empty());
    }
}
