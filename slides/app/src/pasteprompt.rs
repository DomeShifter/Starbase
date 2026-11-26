//! The Paste standard in Slides (crate starbase-clipboard, docs/decisions.md 2026-11-26): a menu
//! Paste (Edit, the ribbon, the slide's right-click menu) pastes what Ctrl+V would. DeckCraft's
//! menus call `Services::menu_paste` first (our fork's one hook); this file answers it:
//! our own copy while the tab kept the focus (the engine pastes it), else the clipboard read on
//! https (or directly on the desktop), else the "Press Ctrl+V now" card, which Ctrl+V then
//! satisfies. Text read here goes back to the engine as `InboxEvent::Paste`.

use deckcraft_ui_egui::{Inbox, InboxEvent};
use starbase_clipboard as clip;

/// `Services::copied`: whatever went to the system clipboard from a Copy or Cut is our own copy.
pub fn copied_hook() -> Box<dyn FnMut(&str)> {
    Box::new(clip::remember)
}

fn push(inbox: &Inbox, text: String) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(InboxEvent::Paste(text));
}

/// `Services::menu_paste`: `true` = handled here (a read is under way or the card shows),
/// `false` = the engine pastes its own copy.
#[cfg(not(target_arch = "wasm32"))]
pub fn menu_paste_hook(inbox: Inbox) -> Box<dyn FnMut() -> bool> {
    // The desktop program reads the system clipboard directly (no focus tracking there).
    Box::new(move || match clip::read_native() {
        Some(text) => {
            push(&inbox, text);
            true
        }
        None => false,
    })
}

/// `Services::menu_paste`: `true` = handled here (a read is under way or the card shows),
/// `false` = the engine pastes its own copy.
#[cfg(target_arch = "wasm32")]
pub fn menu_paste_hook(inbox: Inbox, ctx: egui::Context) -> Box<dyn FnMut() -> bool> {
    Box::new(move || match clip::decide(clip::own_copy().as_deref(), clip::can_read(), false) {
        clip::PasteRoute::Own(_) => false,
        clip::PasteRoute::Read => {
            read(inbox.clone(), ctx.clone());
            true
        }
        clip::PasteRoute::Prompt => {
            clip::show_prompt();
            ctx.request_repaint();
            true
        }
    })
}

/// Reads the clipboard (https); a refused or empty read shows the card.
#[cfg(target_arch = "wasm32")]
fn read(inbox: Inbox, ctx: egui::Context) {
    let Some(w) = web_sys::window() else { return };
    let nav = w.navigator();
    if js_sys::Reflect::get(&nav, &"clipboard".into()).is_ok_and(|c| c.is_undefined() || c.is_null()) {
        clip::show_prompt();
        return;
    }
    let promise = nav.clipboard().read_text();
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|v| v.as_string()).filter(|t| !t.is_empty()) {
            Some(text) => push(&inbox, text),
            None => clip::show_prompt(),
        }
        ctx.request_repaint();
    });
}

/// Call before the slide each frame: a Ctrl+V (the browser's paste) or Esc this frame closes the
/// card. Esc is consumed only while the card shows.
pub fn before(ctx: &egui::Context) {
    if !clip::prompt_shown() {
        return;
    }
    let pasted = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    clip::prompt_frame(pasted, esc);
}

/// Call after everything else: draws the card while it waits.
pub fn ui(ctx: &egui::Context) {
    if !clip::prompt_shown() {
        return;
    }
    let mut close = false;
    let area = egui::Area::new(egui::Id::unique("starbase_paste_prompt"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 96.0))
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(360.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(clip::PROMPT_TITLE).size(14.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.small_button("\u{2715}");
                        crate::shell::target(ui.ctx(), "paste prompt: close", r.rect);
                        close = r.clicked();
                    });
                });
                if clip::explain() {
                    ui.label(egui::RichText::new(clip::PROMPT_REASON).size(12.0).weak());
                    let mut off = false;
                    let r = ui.checkbox(&mut off, clip::DONT_EXPLAIN);
                    crate::shell::target(ui.ctx(), "paste prompt: don't explain again", r.rect);
                    if r.changed() && off {
                        clip::set_explain(false);
                    }
                }
            });
        });
    crate::shell::target(ctx, "paste prompt", area.response.rect);
    if close {
        clip::close_prompt();
    }
}
