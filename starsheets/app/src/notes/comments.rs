//! Comments the Google Sheets way: a cell with a comment has an orange corner (GridCraft draws
//! it); hovering over it or selecting it shows the thread as a white card beside the cell, with
//! who wrote each part, a Resolve tick, a ⋮ menu (Edit, Delete) and a Reply box. Insert › Comment
//! (Ctrl+Alt+M) opens an empty card to write a new one. Everything runs GridCraft's `review.*`
//! commands, so comments are undoable, live-synced and saved in the .xlsx as threaded comments.

use egui::{Align, Align2, Color32, Layout, Rect, RichText, Sense, Ui, vec2};
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::Comment;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// Asks for a new comment on the active cell (Insert › Comment, Ctrl+Alt+M), through egui's memory.
pub const REQUEST: &str = "starsheets_comment";
const WIDTH: f32 = 300.0;
const AVATAR: f32 = 28.0;
/// Avatar colours for people who aren't the one signed in (the server hands out the same ones).
const COLOURS: [Color32; 8] = [
    Color32::from_rgb(0xD9, 0x30, 0x25),
    Color32::from_rgb(0x18, 0x80, 0x38),
    Color32::from_rgb(0xE3, 0x74, 0x00),
    Color32::from_rgb(0x93, 0x34, 0xE6),
    Color32::from_rgb(0x00, 0x7B, 0x83),
    Color32::from_rgb(0xE5, 0x25, 0x92),
    Color32::from_rgb(0xB0, 0x60, 0x00),
    Color32::from_rgb(0x5F, 0x63, 0x68),
];

/// What the box on the card is being written for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Writing {
    /// A new comment on the cell.
    New,
    /// The thread's first entry, changed (⋮ › Edit).
    Edit,
    /// A reply to the thread.
    Reply,
}

#[derive(Default)]
pub struct Comments {
    /// The text being written: (document, cell, what for, text).
    draft: Option<(u64, CellRef, Writing, String)>,
    focus: bool,
    /// The card shown last frame (cell, rect): it stays while the pointer is over it, so its
    /// buttons and Reply box can be reached after leaving the cell.
    last: Option<(CellRef, Rect)>,
    /// The selected cell whose card was closed (Esc, or a click elsewhere): it stays closed
    /// until another cell is selected, as in Sheets.
    dismissed: Option<(u64, CellRef)>,
}

/// Whether `c` is a comment (GridCraft keeps notes and comments in one map).
pub fn is_comment(c: &Comment) -> bool {
    c.threaded
}

/// The name comments are signed with: the signed-in person's, else GridCraft's default.
#[cfg(target_arch = "wasm32")]
fn my_name() -> Option<String> {
    crate::account::me().map(|m| if m.name.trim().is_empty() { m.username } else { m.name })
}

/// The desktop app's local mode has no accounts: the name of this computer's user.
#[cfg(not(target_arch = "wasm32"))]
fn my_name() -> Option<String> {
    Some(crate::localstore::os_user_name())
}

/// Who wrote it, as the card shows it.
fn shown_name(author: &str) -> &str {
    if author.trim().is_empty() { "Unknown" } else { author }
}

/// The same colour for the same name every time.
fn colour_for(name: &str) -> Color32 {
    let h = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    COLOURS[h as usize % COLOURS.len()]
}

/// The parameters that sign a command with my name, when there is one.
fn signed(mut p: serde_json::Value) -> serde_json::Value {
    if let Some(name) = my_name() {
        p["author"] = json!(name);
    }
    p
}

impl Comments {
    /// Starts a new comment on the active cell (or editing the one there, as Sheets opens it).
    pub fn start(&mut self, app: &SheetApp) {
        let Some(d) = app.session.active() else { return };
        let at = d.selection.active;
        let has = d.wb.active().and_then(|sh| sh.comments.get(&at)).is_some_and(is_comment);
        self.draft = Some((d.uid, at, if has { Writing::Reply } else { Writing::New }, String::new()));
        self.focus = true;
    }

    /// True while a comment is being written (the note card then stays away).
    pub fn writing(&self) -> bool {
        self.draft.is_some()
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let Some(d) = app.session.active() else { return };
        let uid = d.uid;
        let Some(sh) = d.wb.active() else { return };
        let has = |c: &CellRef| sh.comments.get(c).is_some_and(is_comment);
        // A draft for another file or sheet's cell that is gone is dropped.
        if self.draft.as_ref().is_some_and(|(doc, c, w, _)| *doc != uid || (*w != Writing::New && !has(c))) {
            self.draft = None;
        }
        let drafting = self.draft.as_ref().map(|(_, c, ..)| *c);
        let pointer = ui.ctx().pointer_hover_pos();
        let over_card = self.last.filter(|(c, r)| has(c) && pointer.is_some_and(|p| r.contains(p))).map(|l| l.0);
        self.last = None;
        let hovered = app.grid.hover_cell.filter(&has);
        if self.dismissed.is_some_and(|x| x != (uid, d.selection.active)) {
            self.dismissed = None;
        }
        let selected = Some(d.selection.active).filter(|c| has(c) && app.editor.is_none() && self.dismissed != Some((uid, *c)));
        // No card over a dialog (Keyboard shortcuts, Custom currency…): it would sit on top of it.
        if crate::input::modal_showing(ui.ctx()) {
            return;
        }
        let Some(cell) = drafting.or(over_card).or(hovered).or(selected) else { return };
        let from_selection = drafting.is_none() && over_card.is_none() && hovered.is_none();
        let thread = sh.comments.get(&cell).filter(|c| is_comment(c)).cloned();
        let Some(cell_rect) = crate::linkcard::cell_rect(app, cell) else { return };
        if app.grid.cells_rect.is_some_and(|r| !r.intersects(cell_rect)) {
            return;
        }
        let mut action: Option<(&'static str, serde_json::Value)> = None;
        let mut start: Option<(Writing, String)> = None;
        let mut cancel = false;
        let at = cell_rect.right_top() + vec2(6.0, 0.0);
        let area = egui::Area::new(egui::Id::unique("starsheets_comment_card"))
            .fixed_pos(at)
            .constrain(false)
            .pivot(Align2::LEFT_TOP)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                house::card_frame().show(ui, |ui| {
                    ui.set_width(WIDTH);
                    // The house's fields: a thin grey outline, 2 px blue while typing.
                    house::form_style(ui);
                    ui.spacing_mut().item_spacing = vec2(house::S, house::S);
                    if let Some(c) = &thread {
                        let resolved = c.resolved;
                        // The thread's first entry with the tick and the ⋮ menu.
                        let (r, hit) = header(ui, &c.author, if resolved { Some("Resolved") } else { None }, true);
                        if r {
                            action = Some(("review.resolveComment", json!({ "cell": cell.a1(), "resolved": !resolved })));
                        }
                        match hit {
                            Some(Menu::Edit) => start = Some((Writing::Edit, c.text.clone())),
                            Some(Menu::Delete) => action = Some(("review.deleteComment", json!({ "cell": cell.a1() }))),
                            None => {}
                        }
                        let editing = self.draft.as_ref().is_some_and(|(_, dc, w, _)| *dc == cell && *w == Writing::Edit);
                        if !editing {
                            body(ui, &c.text);
                        }
                        for (who, text) in &c.replies {
                            ui.add_space(house::XS);
                            header(ui, who, None, false);
                            body(ui, text);
                        }
                        if resolved {
                            return;
                        }
                    } else {
                        header(ui, &my_name().unwrap_or_default(), None, false);
                    }
                    // The box: a new comment, an edit of the first entry, or a reply.
                    let writing = self.draft.as_ref().filter(|(_, dc, ..)| *dc == cell).map(|(_, _, w, _)| *w);
                    let what = writing.unwrap_or(if thread.is_some() { Writing::Reply } else { Writing::New });
                    let mut text = self.draft.as_ref().filter(|(_, dc, ..)| *dc == cell).map(|d| d.3.clone()).unwrap_or_default();
                    let hint = match what {
                        Writing::New => "Comment",
                        Writing::Edit => "Edit comment",
                        Writing::Reply => "Reply",
                    };
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .hint_text(hint)
                            .desired_width(WIDTH)
                            .desired_rows(if writing.is_some() { 2 } else { 1 })
                            .margin(vec2(house::M, house::S))
                            .font(egui::FontId::proportional(house::BODY)),
                    );
                    if self.focus && writing.is_some() {
                        r.request_focus();
                        self.focus = false;
                    }
                    if r.changed() || (r.has_focus() && writing.is_none()) {
                        self.draft = Some((uid, cell, what, text.clone()));
                    }
                    let send_keys = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                    if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    if writing.is_some() || what == Writing::New {
                        let ready = !text.trim().is_empty();
                        let main = match what {
                            Writing::New => "Comment",
                            Writing::Edit => "Save",
                            Writing::Reply => "Reply",
                        };
                        let mut send = send_keys && ready;
                        ui.allocate_ui_with_layout(vec2(WIDTH, house::BUTTON_HEIGHT), Layout::right_to_left(Align::Center), |ui| {
                            // Not ready: Google's grey, not a faded blue.
                            let button = if ready { house::primary_in(main, house::BLUE) } else { house::primary_off(main) };
                            send |= ui.add_enabled(ready, button).clicked();
                            if ui.add(house::secondary_in("Cancel", house::BLUE)).clicked() {
                                cancel = true;
                            }
                        });
                        if send {
                            let p = json!({ "cell": cell.a1(), "text": text.trim() });
                            action = Some(match what {
                                Writing::New => ("review.newComment", signed(p)),
                                // Editing keeps who wrote it.
                                Writing::Edit => ("review.newComment", p),
                                Writing::Reply => ("review.replyComment", signed(p)),
                            });
                        }
                    }
                });
            });
        let card = area.response.rect;
        self.last = Some((cell, card.expand(8.0)));
        if let Some((w, text)) = start {
            self.draft = Some((uid, cell, w, text));
            self.focus = true;
        }
        // A press outside the card drops an empty draft; one with text in it stays, as in Sheets.
        let pressed_away = ui.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !card.contains(p)));
        if pressed_away && self.draft.as_ref().is_some_and(|d| d.3.trim().is_empty()) {
            cancel = true;
        }
        // Esc, or a click away from both the card and its cell, closes a card shown because its
        // cell is selected.
        let away = |p: egui::Pos2| !card.contains(p) && !cell_rect.contains(p);
        let away_from_cell = ui.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(away));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if from_selection && self.draft.is_none() && (escape || away_from_cell) {
            self.dismissed = Some((uid, cell));
        }
        if cancel {
            self.draft = None;
        }
        if let Some((cmd, p)) = action {
            self.draft = None;
            app.run_or_alert(cmd, p);
        }
    }
}

#[derive(Clone, Copy)]
enum Menu {
    Edit,
    Delete,
}

/// One entry's top line: avatar and name, and for the thread's first entry the Resolve tick (or
/// "Re-open" once resolved) and the ⋮ menu. Returns (tick clicked, menu choice).
fn header(ui: &mut Ui, author: &str, tag: Option<&str>, controls: bool) -> (bool, Option<Menu>) {
    let mut tick = false;
    let mut hit = None;
    ui.horizontal(|ui| {
        let name = shown_name(author);
        let (rect, _) = ui.allocate_exact_size(vec2(AVATAR, AVATAR), Sense::hover());
        ui.painter().circle_filled(rect.center(), AVATAR / 2.0, colour_for(name));
        let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, egui::FontId::proportional(AVATAR * 0.5), Color32::WHITE);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(name).size(house::BODY).strong().color(theme::TEXT));
            if let Some(t) = tag {
                ui.label(RichText::new(t).size(house::NOTE).color(theme::TEXT_DIM));
            }
        });
        if !controls {
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let more = ui
                .add(egui::Button::new(RichText::new("⋮").size(18.0).color(theme::TEXT_DIM)).frame_when_inactive(false))
                .on_hover_text("More options");
            egui::Popup::menu(&more).id(egui::Id::unique("starsheets_comment_more")).show(|ui| {
                crate::shell::menu_style_width(ui, 160.0);
                if house::menu_row(ui, None, "Edit", "", tag.is_none()) {
                    hit = Some(Menu::Edit);
                    ui.close();
                }
                if house::menu_row(ui, None, "Delete", "", true) {
                    hit = Some(Menu::Delete);
                    ui.close();
                }
            });
            let (label, tip) = if tag.is_some() { ("↺", "Re-open") } else { ("✓", "Mark as resolved and hide discussion") };
            tick =
                ui.add(egui::Button::new(RichText::new(label).size(18.0).color(house::BLUE)).frame_when_inactive(false)).on_hover_text(tip).clicked();
        });
    });
    (tick, hit)
}

/// An entry's text.
fn body(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(house::BODY).color(theme::TEXT)).wrap().selectable(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the card's buttons run: a new comment, a reply, an edit that keeps the author,
    /// resolve and re-open, delete; and the note card leaves comments alone.
    #[test]
    fn comment_commands_build_a_thread() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("review.newComment", json!({ "cell": "B2", "text": "Is this right?", "author": "Ann" })).unwrap();
        s.execute("review.replyComment", json!({ "cell": "B2", "text": "Yes", "author": "Bob" })).unwrap();
        s.execute("review.newComment", json!({ "cell": "B2", "text": "Is this total right?" })).unwrap();
        let c = |s: &gridcraft_engine::Session| s.active().unwrap().wb.active().unwrap().comments.get(&CellRef::new(1, 1)).cloned().unwrap();
        let t = c(&s);
        assert!(is_comment(&t));
        assert_eq!((t.author.as_str(), t.text.as_str()), ("Ann", "Is this total right?"), "an edit keeps who wrote it");
        assert_eq!(t.replies, vec![("Bob".to_string(), "Yes".to_string())]);
        s.execute("review.resolveComment", json!({ "cell": "B2", "resolved": true })).unwrap();
        assert!(c(&s).resolved);
        s.execute("review.resolveComment", json!({ "cell": "B2", "resolved": false })).unwrap();
        assert!(!c(&s).resolved);
        s.execute("review.newNote", json!({ "cell": "C3", "text": "a note" })).unwrap();
        let note = s.active().unwrap().wb.active().unwrap().comments.get(&CellRef::new(2, 2)).cloned().unwrap();
        assert!(!is_comment(&note), "notes stay notes");
        s.execute("review.deleteComment", json!({ "cell": "B2" })).unwrap();
        assert!(s.active().unwrap().wb.active().unwrap().comments.get(&CellRef::new(1, 1)).is_none());
    }

    #[test]
    fn names_get_steady_colours() {
        assert_eq!(colour_for("Ann"), colour_for("Ann"));
        assert_eq!(shown_name(""), "Unknown");
    }

    /// The desktop app signs comments with the computer user's name, never GridCraft's default.
    #[test]
    fn local_comments_are_signed_with_the_users_name() {
        let me = crate::localstore::os_user_name();
        assert_eq!(signed(json!({ "text": "x" }))["author"], json!(me));
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("review.newComment", signed(json!({ "cell": "A1", "text": "Hi" }))).unwrap();
        let c = s.active().unwrap().wb.active().unwrap().comments.get(&CellRef::new(0, 0)).cloned().unwrap();
        assert_eq!(c.author, me);
        assert_ne!(c.author, gridcraft_engine::Prefs::default().user_name);
    }
}
