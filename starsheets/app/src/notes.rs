//! Notes the Google Sheets way: a cell with a note has a small black corner (GridCraft draws it);
//! hovering over it or selecting it shows the note as a white card beside the cell. Insert › Note
//! (or a click on the card) edits it there; clicking away saves it, and an emptied note is
//! deleted. The grip in the card's bottom-right corner resizes it, and the size is kept in the
//! file. Comments (threaded, with replies) have their own card: `comments.rs`.

use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, vec2};
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{NoteBox, NoteCorner};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

mod comments;

/// Asks for a new comment on the active cell (Insert › Comment, Ctrl+Alt+M).
pub use comments::REQUEST as COMMENT_REQUEST;

/// Asks to edit the active cell's note (Insert › Note), through egui's memory.
pub const REQUEST: &str = "starsheets_note";
const DEFAULT: (f32, f32) = (230.0, 100.0);
const MIN: (f32, f32) = (120.0, 48.0);
const TEXT: Color32 = Color32::from_rgb(0x3C, 0x40, 0x43);

#[derive(Default)]
pub struct Notes {
    /// The note being written: (document, cell, text).
    editing: Option<(u64, CellRef, String)>,
    /// A size being dragged with the grip: (cell, width, height).
    resizing: Option<(CellRef, f32, f32)>,
    /// Where that drag began: (pointer, width, height). Sizes come from this, not from per-frame
    /// deltas, which egui can apply twice when a frame is laid out twice.
    drag_start: Option<(Pos2, f32, f32)>,
    focus: bool,
    /// The card shown last frame (cell, rect): it stays while the pointer is over it, so it can be
    /// clicked, scrolled or resized after leaving the cell.
    last: Option<(CellRef, Rect)>,
    /// Comment threads, drawn as their own cards.
    comments: comments::Comments,
}

impl Notes {
    /// Takes over GridCraft's note dialog (right-click › Insert note, Shift+F2) and Insert › Note.
    pub fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let asked = ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_some();
        // GridCraft names its form "note" (a note) or "comment" (a threaded comment).
        let comment = ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(COMMENT_REQUEST))).is_some()
            || app.dialog.as_ref().is_some_and(|d| d.name == "comment");
        let dialog = app.dialog.as_ref().is_some_and(|d| d.name == "note");
        if dialog || comment {
            app.dialog = None;
        }
        if comment {
            self.editing = None;
            self.comments.start(app);
            return;
        }
        if (asked || dialog)
            && let Some(d) = app.session.active()
        {
            let at = d.selection.active;
            // A comment's text doesn't carry over: a note written there replaces the comment.
            let text = d.wb.active().and_then(|sh| sh.comments.get(&at)).filter(|c| !comments::is_comment(c)).map(|c| c.text.clone()).unwrap_or_default();
            self.editing = Some((d.uid, at, text));
            self.focus = true;
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut egui::Ui) {
        self.comments.ui(app, ui);
        if self.comments.writing() {
            return;
        }
        let Some(d) = app.session.active() else { return };
        let uid = d.uid;
        let Some(sh) = d.wb.active() else { return };
        // Which note shows: the one being written, else the hovered cell's, else the selected one's.
        let editing = self.editing.as_ref().filter(|(doc, ..)| *doc == uid).map(|(_, c, _)| *c);
        let is_note = |c: &CellRef| sh.comments.get(c).is_some_and(|n| !comments::is_comment(n));
        let hovered = app.grid.hover_cell.filter(&is_note);
        let selected = Some(d.selection.active).filter(|c| is_note(c) && app.editor.is_none());
        let pointer = ui.ctx().pointer_hover_pos();
        let over_card = self.last.filter(|(c, r)| is_note(c) && pointer.is_some_and(|p| r.contains(p))).map(|l| l.0);
        self.last = None;
        let Some(cell) = editing.or(self.resizing.map(|r| r.0)).or(over_card).or(hovered).or(selected) else { return };
        let note = sh.comments.get(&cell);
        let text = note.map(|n| n.text.clone()).unwrap_or_default();
        let (mut w, mut h) =
            note.and_then(|n| n.note_box.as_ref()).and_then(|b| b.size_pt).map_or(DEFAULT, |(w, h)| ((w / 0.75) as f32, (h / 0.75) as f32));
        if let Some((c, rw, rh)) = self.resizing
            && c == cell
        {
            (w, h) = (rw, rh);
        }
        let Some(cell_rect) = crate::linkcard::cell_rect(app, cell) else { return };
        if app.grid.cells_rect.is_some_and(|r| !r.intersects(cell_rect)) {
            return;
        }
        let at = cell_rect.right_top() + vec2(6.0, 0.0);
        let mut commit = false;
        let mut start_edit = false;
        let mut new_size = None;
        let area = egui::Area::new(egui::Id::unique("starsheets_note_card"))
            .fixed_pos(at)
            .constrain(false)
            .pivot(Align2::LEFT_TOP)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                let writing = self.editing.as_ref().is_some_and(|(doc, c, _)| *doc == uid && *c == cell);
                let frame = crate::house::card_frame().inner_margin(egui::Margin::same(crate::house::M as i8)).show(ui, |ui| {
                    ui.set_min_size(vec2(w - 24.0, h - 24.0));
                    ui.set_max_size(vec2(w - 24.0, h - 24.0));
                    if let Some((_, _, draft)) = self.editing.as_mut().filter(|(doc, c, _)| *doc == uid && *c == cell) {
                        let r = ui.add(
                            egui::TextEdit::multiline(draft)
                                .frame(egui::Frame::NONE)
                                .text_color(TEXT)
                                .desired_width(w - 24.0)
                                .desired_rows(3)
                                .font(egui::FontId::proportional(14.0)),
                        );
                        if self.focus {
                            r.request_focus();
                            self.focus = false;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            commit = true;
                        }
                    } else {
                        ui.add(egui::Label::new(RichText::new(&text).size(14.0).color(TEXT)).wrap().selectable(false));
                    }
                });
                // A click anywhere on the card starts writing, as in Sheets.
                if !writing
                    && ui
                        .interact(frame.response.rect, egui::Id::unique("starsheets_note_card_click"), Sense::click())
                        .on_hover_cursor(egui::CursorIcon::Text)
                        .clicked()
                {
                    start_edit = true;
                }
                // The resize grip: three short diagonal lines in the bottom-right corner, on the card's
                // own layer (registered last, so it wins over the text under it).
                let card = ui.min_rect();
                let grip = Rect::from_min_size(card.right_bottom() - vec2(16.0, 16.0), vec2(14.0, 14.0));
                for k in [4.0, 8.0, 12.0] {
                    ui.painter().line_segment(
                        [Pos2::new(grip.right() - k, grip.bottom()), Pos2::new(grip.right(), grip.bottom() - k)],
                        Stroke::new(1.0, Color32::from_gray(0x80)),
                    );
                }
                let g = ui.interact(grip, egui::Id::unique("starsheets_note_grip_drag"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeNwSe);
                (g.dragged(), g.drag_stopped(), g.interact_pointer_pos())
            });
        let card = area.response.rect;
        self.last = Some((cell, card.expand(8.0)));
        let (dragging, stopped, pointer) = area.inner;
        if dragging && let Some(p) = pointer {
            let (from, sw, sh) = *self.drag_start.get_or_insert((p, w, h));
            self.resizing = Some((cell, (sw + p.x - from.x).max(MIN.0), (sh + p.y - from.y).max(MIN.1)));
        }
        if stopped {
            self.drag_start = None;
            if let Some((c, rw, rh)) = self.resizing.take() {
                new_size = Some((c, rw, rh));
            }
        }
        // A press outside the card while writing saves the note.
        if self.editing.is_some() && ui.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !card.contains(p))) {
            commit = true;
        }
        if start_edit {
            self.editing = Some((uid, cell, text.clone()));
            self.focus = true;
        }
        if commit
            && let Some((doc, c, draft)) = self.editing.take()
            && doc == uid
        {
            let old = app.session.active().and_then(|d| d.wb.active()).and_then(|sh| sh.comments.get(&c)).map(|n| n.text.clone());
            if draft.trim().is_empty() {
                if old.is_some() {
                    app.run_or_alert("review.deleteComment", json!({ "cell": c.a1() }));
                }
            } else if old.as_deref() != Some(draft.as_str()) {
                app.run_or_alert("review.newNote", json!({ "cell": c.a1(), "text": draft }));
            }
        }
        if let Some((c, rw, rh)) = new_size
            && let Some(d) = app.session.active_mut()
        {
            let si = d.wb.active_sheet();
            if let Some(note) = std::sync::Arc::make_mut(&mut d.wb).sheet_mut(si).and_then(|sh| sh.comments.get_mut(&c)) {
                note.note_box = Some(note_box(rw, rh));
            }
        }
    }
}

/// A note box of `w` x `h` pixels next to its cell, as the file stores it.
fn note_box(w: f32, h: f32) -> NoteBox {
    NoteBox {
        from: NoteCorner { col: 1, col_px: 15, row: 0, row_px: 2 },
        to: NoteCorner { col: 1 + (w / 100.0).ceil() as i64, col_px: 15, row: (h / 21.0).ceil() as i64, row_px: 2 },
        size_pt: Some((f64::from(w) * 0.75, f64::from(h) * 0.75)),
    }
}
