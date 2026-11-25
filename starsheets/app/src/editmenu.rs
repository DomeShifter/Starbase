//! Right-click inside a cell being edited (or the formula bar while editing): Cut, Copy, Paste,
//! Delete and Select all for the text, as the browser's own menu offers in Google Sheets
//! (to-do 3 row 14). The page is one canvas, so the browser's text menu can't appear there:
//! this is the house version of it.
//!
//! Paste reads the clipboard only where the browser allows a page to (https, after the browser
//! asks); elsewhere it explains Ctrl+V, as Sheets does for its own Edit menu.

use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// The text StarSheets itself last put on the clipboard (cells or text in a cell), while this
    /// tab has kept the focus. On plain http a page can't read the clipboard, so menu Paste
    /// (Edit › Paste, a cell's right-click Paste, this menu's Paste) uses this instead; Ctrl+V
    /// always gets the real clipboard from the browser.
    static LAST_COPY: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Remembers text StarSheets just copied (web.rs: every copy, any route).
#[cfg(target_arch = "wasm32")]
pub fn remember_copy(text: &str) {
    LAST_COPY.with(|c| *c.borrow_mut() = Some(text.replace("\r\n", "\n")));
}

/// The text StarSheets last copied, if the tab hasn't lost the focus since.
pub fn last_copy() -> Option<String> {
    LAST_COPY.with(|c| c.borrow().clone()).filter(|t| !t.is_empty())
}

/// The tab lost the focus: something else may have been copied meanwhile.
#[cfg(target_arch = "wasm32")]
pub fn forget_copy() {
    LAST_COPY.with(|c| *c.borrow_mut() = None);
}

use gridcraft_ui_egui::SheetApp;

use crate::house;

#[derive(Default)]
pub struct EditMenu {
    /// Where the menu opened (the pointer), while it shows.
    open: Option<egui::Pos2>,
    /// Where it was drawn last frame (a click there is a pick, not a click elsewhere).
    rect: Option<egui::Rect>,
    /// Esc was pressed while it showed (taken before the grid sees it).
    escape: bool,
    /// Text read from the clipboard for Paste, inserted on the frame it arrives.
    pasted: Rc<RefCell<Option<String>>>,
    /// The "use Ctrl+V" note, when the browser won't let a page read the clipboard.
    paste_note: bool,
    /// The selection (anchor, caret) as the right button went down. egui's text box moves the
    /// caret to the pointer on ANY press (egui text_cursor_state.rs: `any_pressed`), which
    /// cleared the selection before the menu could cut or copy it (to-do 3 row 14, James's
    /// test): it is put back on every frame until the button is let go.
    kept: Option<(usize, usize)>,
}

/// `text` without the characters `a..b` (character indexes), and that text.
pub fn split_out(text: &str, a: usize, b: usize) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = (a.min(chars.len()), b.min(chars.len()));
    let rest = chars[..a].iter().chain(&chars[b..]).collect();
    (rest, chars[a..b].iter().collect())
}

/// `text` with `insert` in place of characters `a..b`.
pub fn replace(text: &str, a: usize, b: usize, insert: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = (a.min(chars.len()), b.min(chars.len()));
    chars[..a].iter().collect::<String>() + insert + &chars[b..].iter().collect::<String>()
}

/// The selection (start, end) in the text being edited, in characters.
fn selection(app: &SheetApp) -> Option<(usize, usize)> {
    let e = app.editor.as_ref()?;
    Some((e.anchor.min(e.caret), e.anchor.max(e.caret)))
}

/// The box being edited that the pointer is over: the cell editor or the formula bar.
fn over_editor(ctx: &egui::Context, at: egui::Pos2) -> bool {
    let cell = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("gridcraft.cell_editor.rect")));
    let bar = ctx.read_response(egui::Id::unique("gridcraft.formula_bar")).map(|r| r.rect);
    cell.is_some_and(|r| r.contains(at)) || bar.is_some_and(|r| r.contains(at))
}

impl EditMenu {
    /// Call before the grid: Esc closes the menu only, it must not cancel the cell; a right
    /// press on the text keeps its selection (see `kept`).
    pub fn before(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if self.open.is_some() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.escape = true;
        }
        let (pressed, held) = ctx.input(|i| (i.pointer.secondary_pressed(), i.pointer.secondary_down() || i.pointer.secondary_released()));
        let Some(e) = app.editor.as_mut() else {
            self.kept = None;
            return;
        };
        if pressed && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| over_editor(ctx, p)) {
            self.kept = Some((e.anchor, e.caret));
        }
        match self.kept {
            // Given back to the text box after it has handled the press (formula_bar.rs applies
            // anchor..caret when keep_selection is set), so the highlight never goes.
            Some((anchor, caret)) if held => {
                e.anchor = anchor;
                e.caret = caret;
                e.keep_selection = true;
                e.request_focus = true;
            }
            _ => self.kept = None,
        }
    }

    /// Call after the grid.
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if app.editor.is_none() {
            self.open = None;
            self.rect = None;
            self.pasted.borrow_mut().take();
            return;
        }
        if let Some(text) = self.pasted.borrow_mut().take() {
            insert(app, &text);
        }
        self.note_ui(ctx);
        let (right, left, at) = ctx.input(|i| (i.pointer.secondary_clicked(), i.pointer.primary_clicked(), i.pointer.interact_pos()));
        let on_menu = at.zip(self.rect).is_some_and(|(p, r)| r.contains(p));
        if right && let Some(p) = at.filter(|p| over_editor(ctx, *p)) {
            self.open = Some(p);
        } else if (left || right) && !on_menu {
            self.open = None;
        }
        let Some(pos) = self.open else {
            self.rect = None;
            return;
        };
        let has_sel = selection(app).is_some_and(|(a, b)| a < b);
        let mut pick = None;
        let shown = egui::Area::new(egui::Id::unique("starsheets_edit_menu")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(0, house::S as i8)).show(ui, |ui| {
                house::menu(ui, 260.0);
                for (k, (icon, label, keys, on)) in [
                    ("content_cut", "Cut", "Ctrl+X", has_sel),
                    ("content_copy", "Copy", "Ctrl+C", has_sel),
                    ("content_paste", "Paste", "Ctrl+V", true),
                    ("delete", "Delete", "", has_sel),
                ]
                .into_iter()
                .enumerate()
                {
                    if house::menu_row(ui, Some(icon), label, keys, on) {
                        pick = Some(k);
                    }
                }
                house::menu_sep(ui);
                if house::menu_row(ui, Some(""), "Select all", "Ctrl+A", true) {
                    pick = Some(4);
                }
            });
        });
        self.rect = Some(shown.response.rect);
        if std::mem::take(&mut self.escape) {
            self.open = None;
        }
        let Some(k) = pick else { return };
        self.open = None;
        self.rect = None;
        let Some((a, b)) = selection(app) else { return };
        let Some(e) = app.editor.as_mut() else { return };
        let (rest, chosen) = split_out(&e.text, a, b);
        match k {
            0 | 3 => {
                if k == 0 {
                    ctx.copy_text(chosen);
                }
                e.text = rest;
                e.caret = a;
                e.anchor = a;
                e.sync_caret = true;
            }
            1 => {
                ctx.copy_text(chosen);
                e.keep_selection = true;
            }
            2 => self.paste(ctx),
            _ => {
                e.anchor = 0;
                e.caret = e.text.chars().count();
                e.keep_selection = true;
            }
        }
        e.request_focus = true;
    }

    /// Paste: the clipboard's text where the browser lets the page read it, else the note.
    fn paste(&mut self, ctx: &egui::Context) {
        // StarSheets' own last copy: still what's on the clipboard, as the tab never lost focus.
        if let Some(text) = last_copy() {
            *self.pasted.borrow_mut() = Some(text);
            ctx.request_repaint();
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(w) = web_sys::window().filter(|w| w.is_secure_context()) {
            let clipboard = w.navigator().clipboard();
            if !(clipboard.is_undefined() || clipboard.is_null()) {
                let (slot, ctx) = (self.pasted.clone(), ctx.clone());
                let promise = clipboard.read_text();
                wasm_bindgen_futures::spawn_local(async move {
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        Ok(v) => {
                            *slot.borrow_mut() = v.as_string();
                            ctx.request_repaint();
                        }
                        Err(e) => log::warn!("Clipboard read refused: {e:?}"),
                    }
                });
                return;
            }
        }
        let _ = ctx;
        self.paste_note = true;
    }

    /// Sheets' note when the browser won't let a menu paste.
    fn note_ui(&mut self, ctx: &egui::Context) {
        if !self.paste_note {
            return;
        }
        let mut closed = false;
        let mut ok = false;
        house::dialog(ctx, "starsheets_paste_note", 420.0, "Copying and pasting in this browser", Some(&mut closed), |ui| {
            house::text(ui, PASTE_NOTE);
            ok = house::actions(ui, &["OK"]).is_some() || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        });
        if closed || ok {
            self.paste_note = false;
        }
    }
}

/// Why a menu can't paste something copied elsewhere (or before a reload) on plain http.
pub const PASTE_NOTE: &str = "The browser doesn't let a menu paste something copied in another app (or before the page was reloaded) on this connection. Use Ctrl+V to paste it (Ctrl+C copies, Ctrl+X cuts).";

/// Puts `text` in place of the selection in the text being edited.
fn insert(app: &mut SheetApp, text: &str) {
    let Some((a, b)) = selection(app) else { return };
    let Some(e) = app.editor.as_mut() else { return };
    e.text = replace(&e.text, a, b, text);
    e.caret = a + text.chars().count();
    e.anchor = e.caret;
    e.sync_caret = true;
    e.request_focus = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_and_paste_work_in_characters() {
        assert_eq!(split_out("héllo wörld", 2, 6), ("héwörld".to_string(), "llo ".to_string()));
        assert_eq!(replace("héllo", 1, 2, "e"), "hello");
        assert_eq!(replace("abc", 3, 3, "d"), "abcd");
        assert_eq!(split_out("abc", 5, 9), ("abc".to_string(), String::new()));
    }
}
