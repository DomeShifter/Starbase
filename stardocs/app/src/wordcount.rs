//! The live word count badge, as Google Docs' Tools › Word count › "Display word count while
//! typing": a small box at the bottom left showing "1,234 words", or "12 of 1,234 words" while
//! text is selected. Clicking it opens the full Word count dialog.
//!
//! Counting a long document takes time, so it is recounted at most once every 750 ms, and only
//! when the document or the selection may have changed.

use egui::{Color32, RichText, vec2};
use serde_json::json;
use wordcraft_ui_egui::WordApp;

use crate::shell::{DIVIDER, TEXT};

/// The shortest time between two counts, in seconds.
const EVERY: f64 = 0.75;

#[derive(Default)]
pub struct WordCount {
    /// The text shown, once counted.
    shown: Option<String>,
    /// When the last count was made (`InputState::time`).
    counted_at: f64,
    /// Something may have changed since the last count.
    stale: bool,
    /// The document revision the shown count was made from.
    rev: u64,
}

/// 1234567 as "1,234,567".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn words(n: u64) -> String {
    format!("{} {}", thousands(n), if n == 1 { "word" } else { "words" })
}

/// The engine's Word count: the selection's words when text is selected, else the document's.
fn count(app: &mut WordApp) -> Option<u64> {
    // The engine directly: through `WordApp::run`, a parameterless review.wordCount opens the dialog.
    app.session.run("review.wordCount", &json!({})).ok()?["words"].as_u64()
}

impl WordCount {
    fn recount(&mut self, app: &mut WordApp) {
        let selected = !app.session.sel.is_collapsed();
        let Some(n) = count(app) else { return };
        let text = if selected {
            // The document's total (cached by the app until the document changes).
            let total = app.cached_status_counts().words as u64;
            format!("{} of {}", thousands(n), words(total))
        } else {
            words(n)
        };
        self.shown = Some(text);
        self.rev = app.session.rev();
    }

    /// A small rounded white box with a soft shadow at the bottom-left of the window showing
    /// "1,234 words", or "12 of 1,234 words" when text is selected. Clicking it opens the full Word
    /// count dialog.
    pub fn ui(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        // Typing, clicking, dragging a selection or a command run elsewhere: count again soon.
        // Plain mouse movement changes nothing, so it doesn't count.
        let (now, input) = ctx.input(|i| {
            let typed = i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut | egui::Event::PointerButton { .. }
                )
            });
            (i.time, typed || i.pointer.any_down())
        });
        if input || self.shown.is_none() || app.session.rev() != self.rev {
            self.stale = true;
        }
        if self.stale {
            let wait = self.counted_at + EVERY - now;
            if self.shown.is_none() || wait <= 0.0 {
                self.stale = false;
                self.counted_at = now;
                self.recount(app);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            }
        }
        let Some(text) = &self.shown else { return };
        let frame = egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(egui::Stroke::new(1.0, DIVIDER))
            .corner_radius(8.0)
            .inner_margin(vec2(12.0, 6.0))
            .shadow(egui::Shadow { offset: [0, 1], blur: 6, spread: 0, color: Color32::from_black_alpha(40) });
        let clicked = egui::Area::new(egui::Id::unique("stardocs_word_count"))
            .anchor(egui::Align2::LEFT_BOTTOM, vec2(16.0, -16.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let r = frame.show(ui, |ui| ui.label(RichText::new(text).size(13.0).color(TEXT))).response;
                r.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Word count").clicked()
            })
            .inner;
        if clicked {
            crate::shell::run(app, "ui.dialog:wordCount", json!({}));
        }
    }
}
