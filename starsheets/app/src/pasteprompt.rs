//! The "Press Ctrl+V now" prompt of the Paste standard (crate starbase-clipboard, house-style.md
//! "Paste"): a small card at the top of the window, not a modal, so the keyboard stays where
//! the menu was opened (the cell or the text being edited) and Ctrl+V pastes there. It closes
//! itself when the paste arrives, on Esc or on its ✕.

use starbase_clipboard as clip;

use crate::house;

/// Call before the grid each frame: a Ctrl+V (the browser's paste) or Esc this frame closes
/// the prompt. Esc is consumed only while the prompt shows, so it doesn't also cancel the cell.
pub fn before(ctx: &egui::Context) {
    if !clip::prompt_shown() {
        return;
    }
    let pasted = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    clip::prompt_frame(pasted, esc);
}

/// Call after everything else: draws the prompt while it waits.
pub fn ui(ctx: &egui::Context) {
    if !clip::prompt_shown() {
        return;
    }
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("starbase_paste_prompt"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, house::XL * 3.0))
        .interactable(true)
        .show(ctx, |ui| {
            house::card_frame().show(ui, |ui| {
                ui.set_max_width(360.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(clip::PROMPT_TITLE).size(house::LABEL).strong().color(crate::theme::text()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.scope(|ui| house::close_button(ui));
                        house::target(ui.ctx(), "paste prompt: close", r.response.rect);
                        close = r.inner;
                    });
                });
                if clip::explain() {
                    house::note(ui, clip::PROMPT_REASON);
                    let mut off = false;
                    let r = ui.checkbox(&mut off, clip::DONT_EXPLAIN);
                    house::target(ui.ctx(), "paste prompt: don't explain again", r.rect);
                    if r.changed() && off {
                        clip::set_explain(false);
                    }
                }
            });
        });
    house::target(ctx, "paste prompt", area.response.rect);
    if close {
        clip::close_prompt();
    }
}
