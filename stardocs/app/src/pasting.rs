//! The Paste standard in Starbase Docs (crate starbase-clipboard, as in Starbase Sheets; see
//! starsheets/docs/house-style.md "Paste"). Menu Paste (Edit › Paste, the toolbar and the page's
//! right-click menu) comes here through WordCraft's host hook ([`WordApp::set_menu_paste`]):
//! Docs' own copy while it is still current is pasted at once (with its formatting); else, where
//! the browser lets a page read the clipboard (https), it is read; on plain http (or a refused
//! read) the "Press Ctrl+V now" prompt shows, and the key pastes at the caret. The desktop app
//! reads the system clipboard directly and never shows the prompt. Ctrl+V itself brings its text
//! and never comes here (it pastes once, #114).

use std::cell::RefCell;

use serde_json::{Value, json};
use starbase_clipboard::{self as clip, PasteRoute};
use wordcraft_ui_egui::WordApp;

use crate::looks;

thread_local! {
    /// The engine's copied text as last seen, to notice a new copy (`remember_copies`).
    static SEEN: RefCell<String> = const { RefCell::new(String::new()) };
    /// A copy or cut happened this frame (Ctrl+C/X, or our menus): remember it next frame even
    /// when it copied the same text again.
    static COPIED: RefCell<bool> = const { RefCell::new(false) };
    /// The clipboard's text read for a menu Paste (https), waiting to be pasted.
    #[cfg(target_arch = "wasm32")]
    static READ: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The window's context, to wake it when the read finishes.
    #[cfg(target_arch = "wasm32")]
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// WordCraft's host hook for menu Paste (`edit.paste` without text).
pub fn menu_paste(app: &mut WordApp) -> Option<Result<Value, String>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The desktop reads the system clipboard itself (it may hold another program's copy);
        // nothing there: WordCraft pastes its own copy as before.
        clip::read_native().map(|text| app.run("edit.paste", json!({ "text": text })))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let route = clip::decide(clip::own_copy().as_deref(), clip::can_read(), false);
        Some(paste_by(app, route))
    }
}

/// Pastes by the route the Paste standard chose. Never pastes WordCraft's own older copy
/// behind the person's back: without a current copy of ours it reads, or asks for Ctrl+V.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn paste_by(app: &mut WordApp, route: PasteRoute) -> Result<Value, String> {
    match route {
        // The engine pastes its own copy (with formatting) when the text matches it.
        PasteRoute::Own(text) => app.run("edit.paste", json!({ "text": text })),
        PasteRoute::Read => {
            #[cfg(target_arch = "wasm32")]
            read();
            #[cfg(not(target_arch = "wasm32"))]
            clip::show_prompt();
            Ok(json!({ "pending": "clipboardRead" }))
        }
        PasteRoute::Prompt => {
            clip::show_prompt();
            Ok(json!({ "pending": "pasteKey" }))
        }
    }
}

/// Starts the page's clipboard read (https); a refused or empty read shows the prompt.
#[cfg(target_arch = "wasm32")]
fn read() {
    let Some(w) = web_sys::window() else {
        clip::show_prompt();
        return;
    };
    let promise = w.navigator().clipboard().read_text();
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|v| v.as_string()).filter(|t| !t.is_empty()) {
            Some(text) => READ.with(|r| *r.borrow_mut() = Some(text)),
            None => clip::show_prompt(),
        }
        if let Some(ctx) = CTX.with(|c| c.borrow().clone()) {
            ctx.request_repaint();
        }
    });
}

/// A copy or cut through our own menus: remember it even when the text is the same as before.
pub fn copied() {
    COPIED.with(|c| *c.borrow_mut() = true);
}

/// Once a frame, before the page: remembers Docs' own copies, pastes a finished clipboard read,
/// lets Ctrl+V or Esc close the prompt, and draws the prompt while it waits.
pub fn frame(app: &mut WordApp, ctx: &egui::Context) {
    remember_copies(app, ctx);
    #[cfg(target_arch = "wasm32")]
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    #[cfg(target_arch = "wasm32")]
    if let Some(text) = READ.with(|r| r.borrow_mut().take())
        && let Err(e) = app.run("edit.paste", json!({ "text": text }))
    {
        app.status(e);
    }
    if clip::prompt_shown() {
        let pasted = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        // Esc is taken only while the prompt shows, so it doesn't also act on the page.
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        clip::prompt_frame(pasted, esc);
        clip::hold_page_focus();
    }
    prompt_ui(ctx);
}

/// Remembers the engine's copy as Docs' own when it changed, or a copy happened last frame.
fn remember_copies(app: &WordApp, ctx: &egui::Context) {
    let text = &app.session.clipboard_text;
    let again = COPIED.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let changed = SEEN.with(|s| *s.borrow() != *text);
    if (again || changed) && !text.is_empty() {
        clip::remember(text);
    }
    if changed {
        SEEN.with(|s| s.borrow_mut().clone_from(text));
    }
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut))) {
        copied();
    }
}

/// The "Press Ctrl+V now" card at the top of the window: not a dialog, so the caret keeps the
/// keyboard and Ctrl+V pastes there. Closes on the paste, on Esc or on its close button.
fn prompt_ui(ctx: &egui::Context) {
    if !clip::prompt_shown() {
        return;
    }
    let mut close = false;
    let area = egui::Area::new(egui::Id::unique("stardocs_paste_prompt"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
        .interactable(true)
        .show(ctx, |ui| {
            looks::dialog_frame().inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                ui.set_max_width(360.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(clip::PROMPT_TITLE).size(16.0).strong().color(crate::shell::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let x = match crate::icons::icon_px("close", 20.0) {
                            Some(img) => egui::Button::image(img),
                            None => egui::Button::new("×"),
                        };
                        let r = ui.add(x.frame(false));
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
                        crate::shell::target(ui.ctx(), "paste prompt: close", r.rect);
                        close = r.clicked();
                    });
                });
                if clip::explain() {
                    ui.label(egui::RichText::new(clip::PROMPT_REASON).size(13.0).color(crate::shell::TEXT));
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
