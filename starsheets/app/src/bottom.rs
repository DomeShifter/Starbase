//! The bottom bar, Sheets style (docs/sheets-reference.md §1 row 6): **+** adds a sheet, **≡** lists
//! all sheets, then the tabs (click to switch, right-click or ▾ for the tab menu, double-click to
//! rename), and on the right the quick sum of the selection (click to choose what it shows).

use egui::{Align, Button, Frame, Layout, Margin, RichText, Stroke, Ui, vec2};
use gridcraft_engine::model::{Color, Visibility};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{icons, theme};

/// What the quick-sum box shows.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum QuickSum {
    #[default]
    Sum,
    Average,
    Min,
    Max,
    Count,
    CountNumbers,
}

impl QuickSum {
    const ALL: [QuickSum; 6] = [QuickSum::Sum, QuickSum::Average, QuickSum::Min, QuickSum::Max, QuickSum::Count, QuickSum::CountNumbers];

    fn label(self) -> &'static str {
        match self {
            QuickSum::Sum => "Sum",
            QuickSum::Average => "Average",
            QuickSum::Min => "Min",
            QuickSum::Max => "Max",
            QuickSum::Count => "Count",
            QuickSum::CountNumbers => "Count numbers",
        }
    }
}

#[derive(Default)]
pub struct Bottom {
    quick: QuickSum,
    /// Tab being renamed: (index, draft name).
    renaming: Option<(usize, String)>,
    /// The tab being dragged to a new place (its sheet index).
    dragging: Option<usize>,
    /// This frame's tab rectangles, for dropping a dragged tab.
    tab_rects: Vec<(usize, egui::Rect)>,
    /// Each tab's width last frame (by name), so tabs can be laid out in slots while one moves.
    widths: std::collections::HashMap<String, f32>,
    /// Each tab's slide away from its slot (by name): (offset, speed), moved by a spring.
    slide: std::collections::HashMap<String, (f32, f32)>,
    /// A tab just dropped: its name and where its left edge was, so it springs into its new slot.
    settle: Option<(String, f32)>,
    /// The pointer's x when the drag began (egui forgets the press position on release).
    drag_origin: Option<f32>,
    /// A tab whose Delete was clicked, waiting for "Are you sure?" (its name: indexes can move).
    confirm_delete: Option<String>,
    /// The tab strip's scroll position and how far it can go, as of last frame (◀ ▶ buttons).
    tab_offset: f32,
    tab_max: f32,
    /// A slide of the tab strip started by ◀ or ▶: (from, to, start time).
    tab_slide: Option<(f32, f32, f64)>,
    /// How wide the quick sum was last frame: the tabs get the rest (to-do 2 row 28: a fixed
    /// 220 px left a gap when it was short or hidden).
    quick_w: f32,
}

/// How long the tab strip takes to slide after a click on ◀ or ▶ (seconds).
const TAB_SLIDE: f64 = 0.25;

/// Where ◀ or ▶ moves the tab strip (to-do 3 row 3): a page of whole tabs at a time.
/// ▶: the last tab on screen (even partly) becomes the first one shown. ◀: the first tab on
/// screen becomes the last one shown. `tabs` are (left, right) in strip coordinates, in order.
/// A tab wider than the strip still moves on by one tab, so a click always does something.
fn tab_page(tabs: &[(f32, f32)], offset: f32, view: f32, max: f32, forward: bool) -> f32 {
    let target = if forward {
        let last = tabs.iter().rposition(|(l, _)| *l < offset + view - 0.5);
        match last {
            Some(k) if tabs[k].0 > offset + 0.5 => tabs[k].0,
            Some(k) => tabs.get(k + 1).map_or(max, |t| t.0),
            None => max,
        }
    } else {
        let first = tabs.iter().position(|(_, r)| *r > offset + 0.5);
        match first {
            Some(k) if tabs[k].1 - view < offset - 0.5 => tabs[k].1 - view,
            Some(k) if k > 0 => tabs[k - 1].1 - view,
            _ => 0.0,
        }
    };
    target.clamp(0.0, max)
}

/// Where a dragged tab would land (its new position among the tabs): past every tab whose middle
/// its own middle has crossed. Dragging right, a tab only gives way once the dragged tab's middle
/// passes its middle (so the last tab stays put until the dragged one is over it); dragging left,
/// the same from the other side.
fn landing(lefts: &[f32], widths: &[f32], from: usize, moved: f32) -> usize {
    let centre = lefts[from] + widths[from] / 2.0 + moved;
    (0..lefts.len()).filter(|k| *k != from && lefts[*k] + widths[*k] / 2.0 < centre).count()
}

/// How far tab `k` slides to make room while tab `from` is dragged to position `to`: the tabs
/// between move one place towards where the dragged tab came from; the rest stay.
fn make_room(k: usize, from: usize, to: usize, step: f32) -> f32 {
    if from < k && k <= to {
        -step
    } else if to <= k && k < from {
        step
    } else {
        0.0
    }
}

/// The spring the tabs move on: stiffness and damping (a little under critical, so a dropped
/// tab overshoots slightly and pings into place, like icons on a phone's home screen).
const SPRING: (f32, f32) = (420.0, 30.0);

struct Tab {
    index: usize,
    name: String,
    colour: Option<egui::Color32>,
    hidden: bool,
}

fn tabs(app: &SheetApp) -> (Vec<Tab>, usize) {
    let Some(d) = app.session.active() else { return (Vec::new(), 0) };
    let theme = &d.wb.theme;
    let list =
        d.wb.sheets
            .iter()
            .enumerate()
            .map(|(index, sh)| Tab {
                index,
                name: sh.name.clone(),
                colour: sh.tab_color.as_ref().and_then(|c: &Color| c.resolve(theme)).map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)),
                hidden: sh.visibility != Visibility::Visible,
            })
            .collect();
    (list, d.wb.active_sheet.get())
}

fn fmt(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        // Thousands separators, Sheets style.
        let s = format!("{}", v.abs() as i64);
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        if v < 0.0 { format!("-{out}") } else { out }
    } else {
        format!("{v:.2}")
    }
}

impl Bottom {
    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        self.confirm_delete_dialog(app, ui.ctx());
        egui::Panel::bottom("starsheets_bottom")
            .exact_size(40.0)
            .frame(
                Frame::NONE.fill(theme::PAGE).inner_margin(Margin { left: 8, right: 12, top: 4, bottom: 4 }).stroke(Stroke::new(1.0, theme::HOVER)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let (list, active) = tabs(app);
                    // Alt+1: rename the current sheet.
                    if app.editor.is_none()
                        && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::Num1))
                        && let Some(tab) = list.iter().find(|t| t.index == active)
                    {
                        self.renaming = Some((tab.index, tab.name.clone()));
                    }
                    if icon_button(ui, "add", "Add sheet (Shift+F11)").clicked() {
                        app.run_or_alert("home.insertSheet", json!({ "before": list.len() }));
                    }
                    let all = match icons::icon("menu") {
                        Some(img) => ui.menu_button(img, |ui| self.all_sheets(app, ui, &list, active)).response,
                        None => ui.menu_button("≡", |ui| self.all_sheets(app, ui, &list, active)).response,
                    };
                    all.on_hover_text("All sheets (Alt+Shift+K)");
                    ui.add_space(6.0);
                    let quick_width = if self.quick_w > 0.0 { self.quick_w + 12.0 } else { 0.0 };
                    // ◀ ▶ after the tabs when they don't all fit (to-do 2 row 18).
                    let overflow = self.tab_max > 0.5;
                    let arrows_width = if overflow { 2.0 * 30.0 + 2.0 + 6.0 } else { 0.0 };
                    let strip = (ui.available_width() - quick_width - arrows_width).max(80.0);
                    // The tab strip shows only the slim bar, as a sign of where you are: it can't
                    // be grabbed (it would sit under the tabs). Shift+wheel and ◀ ▶ scroll it.
                    ui.spacing_mut().scroll =
                        egui::style::ScrollStyle { bar_width: 2.0, floating_width: 2.0, ..egui::style::ScrollStyle::floating() };
                    let mut area = egui::ScrollArea::horizontal()
                        .max_width(strip)
                        .id_salt("starsheets_tabs")
                        .scroll_source(egui::containers::scroll_area::ScrollSource { scroll_bar: false, ..Default::default() });
                    // A ◀ ▶ slide: ease out, one step a frame.
                    if let Some((from, to, t0)) = self.tab_slide {
                        let t = ((ui.input(|i| i.time) - t0) / TAB_SLIDE).clamp(0.0, 1.0) as f32;
                        let eased = 1.0 - (1.0 - t).powi(3);
                        area = area.horizontal_scroll_offset(from + (to - from) * eased);
                        if t >= 1.0 {
                            self.tab_slide = None;
                        } else {
                            ui.ctx().request_repaint();
                        }
                    }
                    let out = area.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            self.tab_rects.clear();
                            let visible: Vec<&Tab> = list.iter().filter(|t| !t.hidden).collect();
                            self.strip(app, ui, &visible, active, list.len());
                        });
                    });
                    self.tab_offset = out.state.clamped_offset().x;
                    self.tab_max = (out.content_size.x - out.inner_rect.width()).max(0.0);
                    if overflow {
                        ui.add_space(6.0);
                        let left = ui.add_enabled_ui(self.tab_offset > 0.5, |ui| icon_button(ui, "chevron_left", "Scroll sheet tabs left")).inner;
                        let base = out.inner_rect.left() - self.tab_offset;
                        let mut tabs: Vec<(f32, f32)> = self.tab_rects.iter().map(|(_, r)| (r.left() - base, r.right() - base)).collect();
                        tabs.sort_by(|a, b| a.0.total_cmp(&b.0));
                        let (offset, view, max) = (self.tab_offset, out.inner_rect.width(), self.tab_max);
                        let page = |forward: bool| tab_page(&tabs, offset, view, max, forward);
                        let now = ui.input(|i| i.time);
                        if left.clicked() {
                            self.tab_slide = Some((self.tab_offset, page(false), now));
                        }
                        let right = ui
                            .add_enabled_ui(self.tab_offset < self.tab_max - 0.5, |ui| icon_button(ui, "chevron_right", "Scroll sheet tabs right"))
                            .inner;
                        if right.clicked() {
                            self.tab_slide = Some((self.tab_offset, page(true), now));
                        }
                    }
                    let w = ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.quick_sum(app, ui)).inner;
                    if (w - self.quick_w).abs() > 0.5 {
                        self.quick_w = w;
                        ui.ctx().request_repaint();
                    }
                });
            });
    }

    fn all_sheets(&mut self, app: &mut SheetApp, ui: &mut Ui, list: &[Tab], active: usize) {
        // Only as wide as the longest name (house rule, house::fit_width).
        let labels: Vec<String> = list.iter().map(|t| if t.hidden { format!("{} (hidden)", t.name) } else { t.name.clone() }).collect();
        let width = crate::house::fit_width(ui, labels.iter().map(String::as_str), true);
        crate::shell::menu_style_width(ui, width);
        for (tab, label) in list.iter().zip(labels) {
            // Sheets ticks the current sheet.
            let row = match (tab.index == active, icons::icon("check")) {
                (true, Some(tick)) => Button::image_and_text(tick, RichText::new(label).size(14.0).color(theme::TEXT)).shortcut_text(""),
                _ => crate::shell::menu_row(&label, "", true),
            };
            if ui.add_sized([width, 32.0], row.frame_when_inactive(false)).clicked() {
                if tab.hidden {
                    app.run_or_alert("sheet.unhide", json!({ "sheet": tab.index }));
                }
                switch_to(app, tab.index);
                ui.close();
            }
        }
    }

    fn tab(&mut self, app: &mut SheetApp, ui: &mut Ui, tab: &Tab, active: bool, count: usize) {
        if let Some((i, draft)) = &mut self.renaming
            && *i == tab.index
        {
            let r = ui.add(egui::TextEdit::singleline(draft).desired_width(110.0));
            self.tab_rects.push((tab.index, r.rect));
            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            // Enter keeps the name, Esc cancels, a click elsewhere keeps it (as in Sheets). Focus is
            // only asked for while the box doesn't have it, or it could never be left.
            if r.lost_focus() || enter || esc {
                // The Enter is used up here, or the grid would read it as its own.
                ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Enter | egui::Key::Escape, .. })));
                let name = draft.trim().to_string();
                if !esc && !name.is_empty() && name != tab.name {
                    app.run_or_alert("sheet.rename", json!({ "sheet": tab.index, "name": name }));
                }
                self.renaming = None;
                ui.memory_mut(|m| m.surrender_focus(r.id));
            } else if !r.has_focus() {
                r.request_focus();
                // The whole name starts selected, so typing replaces it.
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                    let all = egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(draft.chars().count()));
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), r.id);
                }
            }
            return;
        }
        let colour = if active { theme::BLUE } else { theme::TEXT_DIM };
        let text = RichText::new(tab.name.clone()).size(13.5).color(colour);
        let text = if active { text.strong() } else { text };
        let fill = if active { theme::ACTIVE_TAB } else { egui::Color32::TRANSPARENT };
        // Sheets' tab: the name (click to switch, drag to reorder, double-click to rename) and a
        // small ▾ that opens the tab menu. Right-click anywhere on it opens the same menu once.
        let shown = Frame::NONE.fill(fill).corner_radius(6.0).inner_margin(Margin { left: 6, right: 2, top: 0, bottom: 0 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let name = ui.add(Button::new(text).frame(false).min_size(vec2(56.0, 30.0)).sense(egui::Sense::click_and_drag()));
            let arrow =
                ui.add(Button::new(RichText::new("▾").size(12.0).color(colour)).frame(false).min_size(vec2(20.0, 30.0))).on_hover_text("Sheet menu");
            (name, arrow)
        });
        let (name, arrow) = shown.inner;
        let whole = shown.response.rect;
        self.tab_rects.push((tab.index, whole));
        // The tab colour: a bar along the bottom of the tab, inside its rounded shape.
        if let Some(c) = tab.colour {
            let bar = egui::Rect::from_min_max(egui::pos2(whole.left(), whole.bottom() - 4.5), whole.max);
            ui.painter().rect_filled(bar, egui::CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 }, c);
        }
        if name.drag_started() {
            self.dragging = Some(tab.index);
        }
        // The release that ends a drag isn't a click: dropping a tab doesn't switch to it.
        if (name.clicked() || arrow.clicked()) && !active && self.dragging.is_none() && self.drag_origin.is_none() {
            switch_to(app, tab.index);
        }
        if name.double_clicked() {
            self.renaming = Some((tab.index, tab.name.clone()));
        }
        // One menu per right-click: the name and the arrow each own a different popup.
        name.context_menu(|ui| self.tab_menu(app, ui, tab, count));
        arrow.context_menu(|ui| self.tab_menu(app, ui, tab, count));
        egui::Popup::menu(&arrow).show(|ui| self.tab_menu(app, ui, tab, count));
    }

    /// The tabs, each in its slot. While one is dragged it follows the pointer and the others
    /// slide aside to show where it will land; on release the sheet moves and the tab springs
    /// into its new slot.
    fn strip(&mut self, app: &mut SheetApp, ui: &mut Ui, visible: &[&Tab], active: usize, count: usize) {
        let gap = ui.spacing().item_spacing.x;
        let known = visible.iter().all(|t| self.widths.contains_key(&t.name));
        let ws: Vec<f32> = visible.iter().map(|t| self.widths.get(&t.name).copied().unwrap_or(0.0)).collect();
        // Natural left edges, from last frame's widths.
        let start = ui.cursor().min.x;
        let mut lefts = Vec::with_capacity(visible.len());
        let mut x = start;
        for w in &ws {
            lefts.push(x);
            x += w + gap;
        }
        let pointer = ui.ctx().pointer_latest_pos();
        if self.dragging.is_none() {
            self.drag_origin = None;
        } else if self.drag_origin.is_none() {
            self.drag_origin = ui.input(|i| i.pointer.press_origin()).map(|p| p.x);
        }
        let origin = self.drag_origin.map(|x| egui::pos2(x, 0.0));
        let from = self.dragging.and_then(|i| visible.iter().position(|t| t.index == i));
        // While dragging: how far the tab has moved, and the slot it would land in.
        let drag = match (from, pointer, origin) {
            (Some(fp), Some(p), Some(o)) if known => {
                let moved = p.x - o.x;
                Some((fp, moved, landing(&lefts, &ws, fp, moved)))
            }
            _ => None,
        };
        let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05);
        let mut moving = false;
        let mut deferred = None;
        for (k, tab) in visible.iter().enumerate() {
            if !known {
                self.tab(app, ui, tab, tab.index == active, count);
                continue;
            }
            let w = ws[k];
            let (slot, _) = ui.allocate_exact_size(vec2(w, 30.0), egui::Sense::hover());
            // Where this tab should be, relative to its slot.
            let target = match drag {
                Some((fp, moved, _)) if k == fp => moved,
                Some((fp, _, to)) => make_room(k, fp, to, ws[fp] + gap),
                None => 0.0,
            };
            if let Some((name, left)) = &self.settle
                && *name == tab.name
            {
                self.slide.insert(tab.name.clone(), (left - slot.left(), 0.0));
                self.settle = None;
            }
            let (mut off, mut vel) = self.slide.get(&tab.name).copied().unwrap_or((0.0, 0.0));
            if drag.is_some_and(|(fp, ..)| fp == k) {
                (off, vel) = (target, 0.0);
            } else {
                vel += (SPRING.0 * (target - off) - SPRING.1 * vel) * dt;
                off += vel * dt;
                if (target - off).abs() < 0.3 && vel.abs() < 3.0 {
                    (off, vel) = (target, 0.0);
                }
            }
            moving |= off != target || vel != 0.0;
            self.slide.insert(tab.name.clone(), (off, vel));
            let rect = slot.translate(vec2(off, 0.0));
            if drag.is_some_and(|(fp, ..)| fp == k) {
                deferred = Some((tab, rect));
            } else {
                self.tab_at(app, ui, tab, rect, active, count);
            }
        }
        // The dragged tab last, so it sits above the others, lifted by a shadow.
        if let Some((tab, rect)) = deferred {
            ui.painter()
                .add(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: egui::Color32::from_black_alpha(50) }.as_shape(rect, 6));
            ui.painter().rect_filled(rect, 6.0, egui::Color32::WHITE);
            self.tab_at(app, ui, tab, rect, active, count);
            moving = true;
        }
        // Remember widths (by name) for the next frame's slots.
        for (i, r) in &self.tab_rects {
            if let Some(t) = visible.iter().find(|t| t.index == *i) {
                self.widths.insert(t.name.clone(), r.width());
            }
        }
        if moving {
            ui.ctx().request_repaint();
        }
        if self.dragging.is_some() && ui.input(|i| i.pointer.any_released()) {
            if let Some((fp, moved, to)) = drag {
                let tab = visible[fp];
                // The other tabs already stand where they belong after the move.
                for t in visible {
                    self.slide.insert(t.name.clone(), (0.0, 0.0));
                }
                self.settle = Some((tab.name.clone(), lefts[fp] + moved));
                if to != fp {
                    // Moving a sheet makes it the current one in GridCraft; Sheets stays where you were.
                    let current = app.session.active().and_then(|d| d.wb.active().map(|sh| sh.name.clone()));
                    app.run_or_alert("sheet.move", json!({ "sheet": tab.index, "to": visible[to].index }));
                    if let Some(name) = current
                        && let Some(i) = app.session.active().and_then(|d| d.wb.sheet_index(&name))
                        && app.session.active().is_some_and(|d| d.wb.active_sheet() != i)
                    {
                        app.run_or_alert("sheet.activate", json!({ "sheet": i }));
                    }
                }
            }
            self.dragging = None;
            self.drag_origin = None;
        }
    }

    /// Draws a tab at `rect` (its slot, slid by an offset).
    fn tab_at(&mut self, app: &mut SheetApp, ui: &mut Ui, tab: &Tab, rect: egui::Rect, active: usize, count: usize) {
        // A fixed id per tab: the dragged one is drawn last, and an id that followed drawing order
        // would change mid-drag and drop it.
        // A child that doesn't take part in the strip's layout: the slots stay where they are and
        // only the offset moves the tab (a scope would move the layout cursor to wherever the
        // slid tab ends, pulling every later slot along with it).
        let mut child =
            ui.new_child(egui::UiBuilder::new().id_salt(("starsheets_tab", &tab.name)).max_rect(rect).layout(Layout::left_to_right(Align::Center)));
        child.spacing_mut().item_spacing.x = 0.0;
        self.tab(app, &mut child, tab, tab.index == active, count);
    }

    /// Sheets asks before deleting a sheet (to-do 2 row 9): Cancel, or OK to delete it. Ctrl+Z
    /// still brings it back afterwards.
    fn confirm_delete_dialog(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let Some(name) = self.confirm_delete.clone() else { return };
        let (mut close, mut cancel, mut delete) = (false, false, false);
        crate::house::dialog(ctx, "starsheets_delete_sheet", 420.0, "Delete this sheet?", Some(&mut close), |ui| {
            ui.label(
                RichText::new(format!("\"{name}\" and everything on it will be deleted. You can undo this with Ctrl+Z."))
                    .size(crate::house::BODY)
                    .color(theme::TEXT),
            );
            match crate::house::actions(ui, &["Cancel", "Delete"]) {
                Some(1) => delete = true,
                Some(_) => cancel = true,
                None => {}
            }
        });
        if delete {
            let index = app.session.active().and_then(|d| d.wb.sheets.iter().position(|sh| sh.name == name));
            if let Some(i) = index {
                app.run_or_alert("home.deleteSheet", json!({ "sheet": i }));
            }
        }
        if close || cancel || delete {
            self.confirm_delete = None;
        }
    }

    fn tab_menu(&mut self, app: &mut SheetApp, ui: &mut Ui, tab: &Tab, count: usize) {
        // Sheets' tab menu: plain rows, no icons (docs/sheets-menus.md).
        let width = crate::shell::menu_style(ui, false);
        let row = |label: &str| crate::shell::menu_row(label, "", false).min_size(vec2(width, 32.0));
        let s = tab.index;
        if ui.add_enabled(count > 1, row("Delete")).clicked() {
            self.confirm_delete = Some(tab.name.clone());
            ui.close();
        }
        if ui.add(row("Duplicate")).clicked() {
            app.run_or_alert("sheet.move", json!({ "sheet": s, "to": s + 1, "copy": true }));
            ui.close();
        }
        // Copy to ▸ New spreadsheet / Existing spreadsheet… (copyto.rs does the work).
        let copy = row("Copy to").right_text(egui::containers::menu::SubMenuButton::arrow_atom(Some(theme::TEXT_DIM)));
        egui::containers::menu::SubMenuButton::from_button(copy).ui(ui, |ui| {
            let width = crate::shell::menu_style(ui, false);
            let sub = |label: &str| crate::shell::menu_row(label, "", false).min_size(vec2(width, 32.0));
            for (label, existing) in [("New spreadsheet", false), ("Existing spreadsheet…", true)] {
                if ui.add(sub(label)).clicked() {
                    ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_copy_to"), (s, existing)));
                    ui.close();
                }
            }
        });
        if ui.add(row("Rename")).clicked() {
            self.renaming = Some((s, tab.name.clone()));
            ui.close();
        }
        let colour = row("Change colour").right_text(egui::containers::menu::SubMenuButton::arrow_atom(Some(theme::TEXT_DIM)));
        let now = tab.colour.map(|c| u32::from(c.r()) << 16 | u32::from(c.g()) << 8 | u32::from(c.b()));
        egui::containers::menu::SubMenuButton::from_button(colour).ui(ui, |ui| {
            crate::colourpicker::menu_for(app, ui, "sheet.tabColor", json!({ "sheet": s }), "none", now, false);
        });
        if ui.add(row("Protect sheet")).clicked() {
            ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(crate::protectpanel::REQUEST), s as i64));
            ui.close();
        }
        if ui.add_enabled(count > 1, row("Hide sheet")).clicked() {
            app.run_or_alert("sheet.hide", json!({ "sheet": s }));
            ui.close();
        }
        crate::shell::menu_sep_width(ui, width);
        if ui.add_enabled(s + 1 < count, row("Move right")).clicked() {
            app.run_or_alert("sheet.move", json!({ "sheet": s, "to": s + 1 }));
            ui.close();
        }
        if ui.add_enabled(s > 0, row("Move left")).clicked() {
            app.run_or_alert("sheet.move", json!({ "sheet": s, "to": s - 1 }));
            ui.close();
        }
    }

    /// Draws the quick sum (right to left) and returns its width (0 when there is none).
    fn quick_sum(&mut self, app: &SheetApp, ui: &mut Ui) -> f32 {
        let Some(d) = app.session.active() else { return 0.0 };
        let Some(sh) = d.wb.active() else { return 0.0 };
        // Like Sheets: only when more than one cell is selected.
        let cells: u64 = d.selection.ranges.iter().map(|r| u64::from(r.end.row - r.start.row + 1) * u64::from(r.end.col - r.start.col + 1)).sum();
        if cells < 2 {
            return 0.0;
        }
        let st = gridcraft_engine::display::stats(sh, &d.selection.ranges);
        if st.count == 0 {
            return 0.0;
        }
        let value = |q: QuickSum| -> String {
            match q {
                QuickSum::Sum => fmt(st.sum),
                QuickSum::Average => st.average.map(fmt).unwrap_or_else(|| "—".into()),
                QuickSum::Min if st.numeric > 0 => fmt(st.min),
                QuickSum::Max if st.numeric > 0 => fmt(st.max),
                QuickSum::Min | QuickSum::Max => "—".into(),
                QuickSum::Count => st.count.to_string(),
                QuickSum::CountNumbers => st.numeric.to_string(),
            }
        };
        // A text-only selection shows Count, as Sheets does.
        let shown = if st.numeric == 0 { QuickSum::Count } else { self.quick };
        let label = RichText::new(format!("{}: {}  ▾", shown.label(), value(shown))).size(13.0).color(theme::TEXT);
        ui.menu_button(label, |ui| {
            ui.set_min_width(200.0);
            for q in QuickSum::ALL {
                if ui.add(Button::new(format!("{}: {}", q.label(), value(q))).selected(q == shown)).clicked() {
                    self.quick = q;
                    ui.close();
                }
            }
        })
        .response
        .rect
        .width()
    }
}

fn icon_button(ui: &mut Ui, name: &str, tip: &str) -> egui::Response {
    match icons::icon(name) {
        Some(img) => ui.add(Button::image(img).frame(false).min_size(vec2(30.0, 30.0))),
        None => ui.add(Button::new(name).frame(false).min_size(vec2(30.0, 30.0))),
    }
    .on_hover_text(tip)
}

/// Goes to another sheet. Like Sheets, a cell being typed into is saved first (GridCraft keeps
/// the edit open across sheets, and the next click then jumps back to finish it). A formula
/// being typed stays open, so cells on the other sheet can still be clicked into it.
pub fn switch_to(app: &mut SheetApp, sheet: usize) {
    let formula = app.editor.as_ref().is_some_and(|e| e.text.trim_start().starts_with('='));
    if app.editor.is_some() && !formula && !app.commit_edit(0, 0, false, false) {
        // The entry was refused (GridCraft shows why); stay on this sheet.
        return;
    }
    app.run_or_alert("sheet.activate", json!({ "sheet": sheet }));
}

#[cfg(test)]
mod tab_drag_tests {
    use super::{landing, make_room};

    /// Five tabs 84 px wide, 2 px apart: Sheet1 at 79, Sheet2 at 165 … Sheet5 at 423.
    const LEFTS: [f32; 5] = [79.0, 165.0, 251.0, 337.0, 423.0];
    const WS: [f32; 5] = [84.0; 5];

    /// Which tabs have slid (by how much) when Sheet2 (index 1) has moved `by` px.
    fn slid(by: f32) -> (usize, Vec<f32>) {
        let to = landing(&LEFTS, &WS, 1, by);
        (to, (0..5).filter(|k| *k != 1).map(|k| make_room(k, 1, to, 86.0)).collect())
    }

    #[test]
    fn dragging_right_moves_only_the_tabs_passed() {
        // A little way: nothing moves.
        assert_eq!(slid(30.0), (1, vec![0.0, 0.0, 0.0, 0.0]));
        // Past Sheet3's middle: only Sheet3 moves left.
        assert_eq!(slid(90.0), (2, vec![0.0, -86.0, 0.0, 0.0]));
        // Over Sheet4 but not yet past the last tab's middle: the last tab stays put.
        assert_eq!(slid(200.0), (3, vec![0.0, -86.0, -86.0, 0.0]));
        // Past the last tab's middle: now it moves left to make room at the end.
        assert_eq!(slid(270.0), (4, vec![0.0, -86.0, -86.0, -86.0]));
    }

    #[test]
    fn dragging_left_moves_tabs_right() {
        let to = landing(&LEFTS, &WS, 3, -100.0);
        assert_eq!(to, 2);
        assert_eq!(make_room(2, 3, to, 86.0), 86.0);
        assert_eq!(make_room(1, 3, to, 86.0), 0.0);
    }
}

#[cfg(test)]
mod tab_page_tests {
    use super::tab_page;

    /// Ten tabs 100 px wide, no gaps; the strip shows 250 px (two and a half tabs).
    fn tabs() -> Vec<(f32, f32)> {
        (0..10).map(|k| (k as f32 * 100.0, k as f32 * 100.0 + 100.0)).collect()
    }
    const VIEW: f32 = 250.0;
    const MAX: f32 = 750.0;

    #[test]
    fn right_arrow_brings_the_last_tab_on_screen_to_the_front() {
        // Tabs 0, 1 and half of 2 are shown: tab 2 becomes the first.
        assert_eq!(tab_page(&tabs(), 0.0, VIEW, MAX, true), 200.0);
        // Never past the end.
        assert_eq!(tab_page(&tabs(), 700.0, VIEW, MAX, true), MAX);
    }

    #[test]
    fn left_arrow_brings_the_first_tab_on_screen_to_the_back() {
        // Tabs 4 (from 400) to half of 6 are shown: tab 4 ends at the right edge.
        assert_eq!(tab_page(&tabs(), 400.0, VIEW, MAX, false), 250.0);
        // Never before the start.
        assert_eq!(tab_page(&tabs(), 100.0, VIEW, MAX, false), 0.0);
    }

    #[test]
    fn a_tab_wider_than_the_strip_still_moves_on() {
        let wide = vec![(0.0, 400.0), (400.0, 800.0), (800.0, 1200.0)];
        assert_eq!(tab_page(&wide, 0.0, VIEW, 950.0, true), 400.0);
        assert_eq!(tab_page(&wide, 400.0, VIEW, 950.0, false), 150.0);
    }
}
