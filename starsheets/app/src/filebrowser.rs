//! File › Open: a small Drive-like file window over the server folder (crates/server):
//! a folder path you can click back along, folders then files, search across all folders, sort by
//! name or date, and the usual handling: new folder, new spreadsheet here, rename (F2), copy / cut
//! / paste (Ctrl+C / X / V), drag onto a folder or the path to move, move to bin (Delete), download,
//! and a Bin view with Restore and Delete forever. It only draws and collects [`Action`]s;
//! files.rs carries them out on the server.

use egui::{Align2, Color32, FontId, Key, RichText, Sense, Stroke, Ui, vec2};

use crate::theme;

const W: f32 = 820.0;
const ROW: f32 = 36.0;
const GREEN: Color32 = crate::house::GREEN;
const SELECTED: Color32 = crate::house::SELECTED;

/// A file (by id) or a folder (by path).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Item {
    File(String),
    Folder(String),
}

#[derive(Clone)]
pub struct FileInfo {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub modified: i64,
    pub size: u64,
}

#[derive(Clone)]
pub struct BinInfo {
    pub name: String,
    pub original: String,
    pub deleted: i64,
    pub folder: bool,
}

pub enum Action {
    /// Open a file: (id, in a new tab).
    Open(String, bool),
    NewFolder(String),
    NewSheet(String),
    Rename(Item, String),
    /// Copy (false) or move (true) items into a folder.
    Paste(Vec<Item>, String, bool),
    Duplicate(String),
    Delete(Vec<Item>),
    Download(String, String),
    Restore(String),
    /// Delete forever: one bin entry, or the whole bin.
    Purge(Option<String>),
    ShowBin,
}

#[derive(Default)]
pub struct Browser {
    pub open: bool,
    pub files: Vec<FileInfo>,
    pub folders: Vec<String>,
    pub bin: Vec<BinInfo>,
    pub loading: bool,
    pub in_bin: bool,
    cwd: String,
    selected: Vec<Item>,
    /// The last clicked item, for Shift+click ranges.
    anchor: Option<Item>,
    /// Copied or cut items: (items, cut).
    clipboard: Option<(Vec<Item>, bool)>,
    renaming: Option<(Item, String, bool)>,
    search: String,
    by_date: bool,
    descending: bool,
    dragging: Option<Vec<Item>>,
    confirm_purge: Option<Option<String>>,
    pub error: Option<String>,
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn size_text(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{:.0} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

impl Browser {
    /// Opens the window in `folder` (the open file's folder, or the top).
    pub fn show_in(&mut self, folder: &str) {
        self.open = true;
        self.in_bin = false;
        self.cwd = folder.to_string();
        self.selected.clear();
        self.search.clear();
        self.renaming = None;
        self.loading = true;
    }

    /// Whether a name is being typed (the rename box): then keys go to the text, not the window.
    pub fn typing(&self) -> bool {
        self.renaming.is_some()
    }

    /// The rows shown: folders then files of the current folder, or every match of a search.
    fn rows(&self) -> Vec<(Item, String, String, i64, Option<u64>)> {
        let q = self.search.trim().to_lowercase();
        let mut folders: Vec<(Item, String, String, i64, Option<u64>)> = self
            .folders
            .iter()
            .filter(|f| if q.is_empty() { parent_of(f) == self.cwd } else { name_of(f).to_lowercase().contains(&q) })
            .map(|f| {
                let newest =
                    self.files.iter().filter(|x| x.folder == *f || x.folder.starts_with(&format!("{f}/"))).map(|x| x.modified).max().unwrap_or(0);
                (Item::Folder(f.clone()), name_of(f).to_string(), parent_of(f).to_string(), newest, None)
            })
            .collect();
        let mut files: Vec<(Item, String, String, i64, Option<u64>)> = self
            .files
            .iter()
            .filter(|f| if q.is_empty() { f.folder == self.cwd } else { f.name.to_lowercase().contains(&q) })
            .map(|f| (Item::File(f.id.clone()), f.name.clone(), f.folder.clone(), f.modified, Some(f.size)))
            .collect();
        for v in [&mut folders, &mut files] {
            if self.by_date {
                v.sort_by_key(|r| std::cmp::Reverse(r.3));
            } else {
                v.sort_by_key(|r| r.1.to_lowercase());
            }
            if self.descending {
                v.reverse();
            }
        }
        folders.extend(files);
        folders
    }

    /// The name of the listed file with this id.
    pub fn name_of(&self, id: &str) -> Option<String> {
        self.file(id).map(|f| f.name.clone())
    }

    fn file(&self, id: &str) -> Option<&FileInfo> {
        self.files.iter().find(|f| f.id == id)
    }

    /// Draws the window; returns what the user asked for.
    pub fn ui(&mut self, ctx: &egui::Context, keys: &[egui::Event]) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        let mut open = true;
        let mut close_now = false;
        let title = if self.in_bin { "Bin" } else { "Open a file" };
        let shown = egui::Window::new(title)
            .title_bar(false)
            .frame(crate::house::dialog_frame())
            .id(egui::Id::unique("starsheets_files_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(vec2(W, 520.0))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 80.0))
            .show(ctx, |ui| {
                ui.set_width(W);
                if crate::house::title_row(ui, title) {
                    close_now = true;
                }
                if self.in_bin {
                    self.bin_ui(ui, &mut out);
                } else {
                    self.files_ui(ui, keys, &mut out);
                }
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).size(crate::house::NOTE).color(crate::house::ERROR));
                }
            });
        self.rename_ui(ctx, &mut out);
        // A click outside the window closes it, as with a menu (not while one of its own menus
        // is open, or a rename or drag is under way).
        if let Some(rect) = shown.map(|r| r.response.rect)
            && self.renaming.is_none()
            && self.dragging.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !rect.contains(p)))
        {
            open = false;
        }
        if !open || close_now {
            self.open = false;
            self.dragging = None;
        }
        out
    }

    fn files_ui(&mut self, ui: &mut Ui, keys: &[egui::Event], out: &mut Vec<Action>) {
        // Path (each part is a place to go back to, and a place to drop onto), search.
        let mut drop_target: Option<String> = None;
        let dragging = self.dragging.is_some();
        ui.horizontal(|ui| {
            // Up one folder, as in Dolphin (Backspace does the same).
            let at_top = self.cwd.is_empty();
            let up_button = match crate::icons::icon("arrow_upward") {
                Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                None => egui::Button::new("Up"),
            };
            let up = ui.add_enabled(!at_top, up_button.frame(false).min_size(vec2(30.0, 30.0))).on_hover_text("Up one folder (Backspace)");
            if up.clicked() {
                self.cwd = parent_of(&self.cwd).to_string();
                self.selected.clear();
                self.search.clear();
            }
            if dragging && !at_top && up.contains_pointer() {
                ui.painter().rect_stroke(up.rect, 4.0, Stroke::new(2.0, GREEN), egui::StrokeKind::Outside);
                drop_target = Some(parent_of(&self.cwd).to_string());
            }
            ui.add_space(4.0);
            let mut parts: Vec<(String, String)> = vec![("My files".into(), String::new())];
            let mut acc = String::new();
            for p in self.cwd.split('/').filter(|p| !p.is_empty()) {
                acc = if acc.is_empty() { p.to_string() } else { format!("{acc}/{p}") };
                parts.push((p.to_string(), acc.clone()));
            }
            let last = parts.len() - 1;
            for (i, (label, path)) in parts.into_iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("›").color(theme::TEXT_DIM));
                }
                let r = ui.add(
                    egui::Button::new(RichText::new(&label).size(15.0).color(if i == last { theme::TEXT } else { theme::TEXT_DIM })).frame(false),
                );
                if dragging && r.contains_pointer() {
                    ui.painter().rect_stroke(r.rect, 4.0, Stroke::new(2.0, GREEN), egui::StrokeKind::Outside);
                    drop_target = Some(path.clone());
                }
                if r.clicked() {
                    self.cwd = path;
                    self.selected.clear();
                    self.search.clear();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .id(egui::Id::unique("starsheets_files_search"))
                        .hint_text("🔍  Search all folders")
                        .desired_width(240.0)
                        .margin(vec2(8.0, 6.0)),
                );
            });
        });
        ui.add_space(6.0);
        // Toolbar.
        let pasteable = self.clipboard.is_some();
        ui.horizontal(|ui| {
            // House buttons: their text kept clear of the edges.
            ui.spacing_mut().button_padding = vec2(crate::house::M, crate::house::S);
            ui.spacing_mut().item_spacing.x = crate::house::S;
            if ui.add(crate::house::primary("+  New spreadsheet")).clicked() {
                out.push(Action::NewSheet(self.cwd.clone()));
            }
            let plain = crate::house::secondary;
            if ui.add(plain("New folder")).clicked() {
                out.push(Action::NewFolder(self.cwd.clone()));
            }
            if ui.add_enabled(pasteable, plain("Paste")).on_hover_text("Ctrl+V").clicked() {
                self.paste_into(self.cwd.clone(), out);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(plain("Bin")).clicked() {
                    self.in_bin = true;
                    self.selected.clear();
                    out.push(Action::ShowBin);
                }
            });
        });
        ui.add_space(6.0);
        // Column headings (click to sort).
        let searching = !self.search.trim().is_empty();
        let (h, _) = ui.allocate_exact_size(vec2(W, 26.0), Sense::hover());
        let name_col = egui::Rect::from_min_size(h.min, vec2(W - 300.0, 26.0));
        let date_col = egui::Rect::from_min_size(h.min + vec2(W - 300.0, 0.0), vec2(190.0, 26.0));
        for (r, label, date) in [(name_col, "Name", false), (date_col, "Last modified", true)] {
            let resp = ui.interact(r, ui.scope_id().with(("sort", label)), Sense::click());
            let arrow = if self.by_date == date { if self.descending { " ↑" } else { " ↓" } } else { "" };
            ui.painter().text(
                r.left_center() + vec2(if date { 0.0 } else { 44.0 }, 0.0),
                Align2::LEFT_CENTER,
                format!("{label}{arrow}"),
                FontId::proportional(13.0),
                theme::TEXT_DIM,
            );
            if resp.clicked() {
                if self.by_date == date {
                    self.descending = !self.descending;
                } else {
                    self.by_date = date;
                    self.descending = false;
                }
            }
        }
        ui.painter().text(h.right_center() - vec2(16.0, 0.0), Align2::RIGHT_CENTER, "Size", FontId::proportional(13.0), theme::TEXT_DIM);
        ui.painter().hline(h.x_range(), h.bottom(), Stroke::new(1.0, theme::DIVIDER));
        let rows = self.rows();
        let mut clicked_empty = true;
        egui::ScrollArea::vertical().max_height(340.0).auto_shrink([false, false]).show(ui, |ui| {
            if self.loading && rows.is_empty() {
                ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
            } else if rows.is_empty() {
                ui.label(
                    RichText::new(if searching {
                        "Nothing matches."
                    } else {
                        "This folder is empty. A new spreadsheet saves itself as soon as you type in it."
                    })
                    .color(theme::TEXT_DIM),
                );
            }
            for (i, (item, name, folder, modified, size)) in rows.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(W - 12.0, ROW), Sense::click_and_drag());
                let selected = self.selected.contains(item);
                let p = ui.painter().clone();
                if selected {
                    p.rect_filled(r, 4.0, SELECTED);
                } else if resp.hovered() {
                    p.rect_filled(r, 4.0, theme::MENU_HOVER);
                }
                let is_folder = matches!(item, Item::Folder(_));
                if is_folder && dragging && resp.contains_pointer() && !self.dragging.as_ref().is_some_and(|d| d.contains(item)) {
                    p.rect_stroke(r, 4.0, Stroke::new(2.0, GREEN), egui::StrokeKind::Inside);
                    if let Item::Folder(f) = item {
                        drop_target = Some(f.clone());
                    }
                }
                let icon = if is_folder { "folder_open" } else { "starsheets" };
                if let Some(img) = crate::icons::icon(icon) {
                    img.paint_at(ui, egui::Rect::from_center_size(r.left_center() + vec2(20.0, 0.0), vec2(22.0, 22.0)));
                }
                let cut = self.clipboard.as_ref().is_some_and(|(c, cut)| *cut && c.contains(item));
                let colour = if cut { theme::TEXT_DIM } else { theme::TEXT };
                // Renaming happens in its own dialog (rename_ui), as in Drive.
                p.text(
                    r.left_center() + vec2(44.0, if searching && !folder.is_empty() { -7.0 } else { 0.0 }),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(14.0),
                    colour,
                );
                if searching && !folder.is_empty() {
                    p.text(
                        r.left_center() + vec2(44.0, 9.0),
                        Align2::LEFT_CENTER,
                        format!("in {folder}"),
                        FontId::proportional(11.0),
                        theme::TEXT_DIM,
                    );
                }
                if *modified > 0 {
                    p.text(
                        r.left_center() + vec2(W - 300.0, 0.0),
                        Align2::LEFT_CENTER,
                        crate::files::date_text(*modified),
                        FontId::proportional(13.0),
                        theme::TEXT_DIM,
                    );
                }
                if let Some(s) = size {
                    p.text(r.right_center() - vec2(16.0, 0.0), Align2::RIGHT_CENTER, size_text(*s), FontId::proportional(13.0), theme::TEXT_DIM);
                }
                // Selecting: click, Ctrl+click adds, Shift+click takes a range.
                if resp.clicked() || resp.secondary_clicked() {
                    clicked_empty = false;
                    let m = ui.input(|i| i.modifiers);
                    if resp.secondary_clicked() && selected {
                        // Keep a multi-selection for the menu.
                    } else if m.command {
                        if selected {
                            self.selected.retain(|x| x != item);
                        } else {
                            self.selected.push(item.clone());
                        }
                        self.anchor = Some(item.clone());
                    } else if m.shift
                        && let Some(a) = self.anchor.as_ref().and_then(|a| rows.iter().position(|r| r.0 == *a))
                    {
                        let (lo, hi) = (a.min(i), a.max(i));
                        self.selected = rows[lo..=hi].iter().map(|r| r.0.clone()).collect();
                    } else {
                        self.selected = vec![item.clone()];
                        self.anchor = Some(item.clone());
                    }
                }
                if resp.double_clicked() {
                    self.activate(item, false, out);
                }
                if resp.drag_started() {
                    let set = if selected { self.selected.clone() } else { vec![item.clone()] };
                    self.selected = set.clone();
                    self.dragging = Some(set);
                }
                resp.context_menu(|ui| {
                    clicked_empty = false;
                    self.item_menu(ui, item, out);
                });
            }
            // Space under the rows: a right-click there offers folder actions.
            let rest = ui.available_rect_before_wrap();
            if rest.height() > 4.0 {
                let r = ui.interact(rest, ui.scope_id().with("files_blank"), Sense::click());
                if r.clicked() && clicked_empty {
                    self.selected.clear();
                }
                r.context_menu(|ui| {
                    crate::house::menu(ui, 240.0);
                    if ui.button("New folder").clicked() {
                        out.push(Action::NewFolder(self.cwd.clone()));
                        ui.close();
                    }
                    if ui.button("New spreadsheet").clicked() {
                        out.push(Action::NewSheet(self.cwd.clone()));
                        ui.close();
                    }
                    if ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste")).clicked() {
                        self.paste_into(self.cwd.clone(), out);
                        ui.close();
                    }
                });
            }
        });
        // Dragging: a label follows the pointer; letting go over a folder or the path moves.
        if let Some(items) = self.dragging.clone() {
            if let Some(pos) = ui.ctx().pointer_latest_pos() {
                let label = if items.len() == 1 { self.label_of(&items[0]) } else { format!("{} items", items.len()) };
                egui::Area::new(egui::Id::unique("starsheets_files_drag"))
                    .fixed_pos(pos + vec2(14.0, 10.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(label);
                        });
                    });
            }
            if ui.input(|i| i.pointer.any_released()) {
                self.dragging = None;
                if let Some(to) = drop_target {
                    out.push(Action::Paste(items, to, true));
                }
            }
        }
        self.keys(ui, keys, &rows, out);
    }

    fn label_of(&self, item: &Item) -> String {
        match item {
            Item::Folder(f) => name_of(f).to_string(),
            Item::File(id) => self.file(id).map_or_else(String::new, |f| f.name.clone()),
        }
    }

    /// Double-click / Enter: a folder opens in the window, a file in this tab.
    fn activate(&mut self, item: &Item, new_tab: bool, out: &mut Vec<Action>) {
        match item {
            Item::Folder(f) => {
                self.cwd = f.clone();
                self.selected.clear();
                self.search.clear();
            }
            Item::File(id) => out.push(Action::Open(id.clone(), new_tab)),
        }
    }

    /// The Rename dialog (house style, like Drive's): the name, then Cancel and OK. Enter is OK,
    /// Esc or ✕ is Cancel.
    fn rename_ui(&mut self, ctx: &egui::Context, out: &mut Vec<Action>) {
        let Some((item, _, _)) = self.renaming.clone() else { return };
        let old = self.label_of(&item);
        let old = if matches!(item, Item::File(_)) { old.rsplit_once('.').map_or(old.clone(), |(s, _)| s.to_string()) } else { old };
        let mut close = false;
        let mut ok = false;
        let mut cancel = false;
        let Some((_, text, focus)) = self.renaming.as_mut() else { return };
        crate::house::dialog(ctx, "starsheets_files_rename", 380.0, "Rename", Some(&mut close), |ui| {
            let te = ui.add_sized(vec2(ui.available_width(), 36.0), crate::house::field(text, crate::house::BODY).text_color(theme::TEXT));
            if *focus {
                te.request_focus();
                *focus = false;
            }
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                ok = true;
            }
            match crate::house::actions(ui, &["Cancel", "OK"]) {
                Some(0) => cancel = true,
                Some(_) => ok = true,
                None => {}
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if ok {
            let t = text.trim().to_string();
            if !t.is_empty() && t != old {
                out.push(Action::Rename(item, t));
            }
            self.renaming = None;
        } else if close || cancel {
            self.renaming = None;
        }
    }

    fn start_rename(&mut self, item: &Item) {
        let name = self.label_of(item);
        // A file is renamed without its extension, as in Drive.
        let shown = match item {
            Item::File(_) => name.rsplit_once('.').map_or(name.clone(), |(s, _)| s.to_string()),
            Item::Folder(_) => name,
        };
        self.renaming = Some((item.clone(), shown, true));
    }

    fn paste_into(&mut self, folder: String, out: &mut Vec<Action>) {
        if let Some((items, cut)) = self.clipboard.clone() {
            out.push(Action::Paste(items, folder, cut));
            if cut {
                self.clipboard = None;
            }
        }
    }

    fn item_menu(&mut self, ui: &mut Ui, item: &Item, out: &mut Vec<Action>) {
        crate::house::menu(ui, 260.0);
        let targets = if self.selected.contains(item) { self.selected.clone() } else { vec![item.clone()] };
        let single = targets.len() == 1;
        let row =
            |ui: &mut Ui, label: &str, keys: &str, enabled: bool| ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys)).clicked();
        match item {
            Item::File(id) => {
                if row(ui, "Open", "Enter", single) {
                    out.push(Action::Open(id.clone(), false));
                    ui.close();
                }
                if row(ui, "Open in new tab", "", single) {
                    out.push(Action::Open(id.clone(), true));
                    ui.close();
                }
            }
            Item::Folder(_) => {
                if row(ui, "Open", "Enter", single) {
                    self.activate(item, false, out);
                    ui.close();
                }
            }
        }
        crate::house::menu_sep(ui);
        if row(ui, "Rename", "F2", single) {
            self.start_rename(item);
            ui.close();
        }
        if let Item::File(id) = item
            && row(ui, "Make a copy", "", single)
        {
            out.push(Action::Duplicate(id.clone()));
            ui.close();
        }
        if let Item::File(id) = item
            && row(ui, "Download", "", single)
        {
            out.push(Action::Download(id.clone(), self.label_of(item)));
            ui.close();
        }
        crate::house::menu_sep(ui);
        if row(ui, "Copy", "Ctrl+C", true) {
            self.clipboard = Some((targets.clone(), false));
            ui.close();
        }
        if row(ui, "Cut", "Ctrl+X", true) {
            self.clipboard = Some((targets.clone(), true));
            ui.close();
        }
        if let Item::Folder(f) = item
            && row(ui, "Paste into folder", "", self.clipboard.is_some() && single)
        {
            self.paste_into(f.clone(), out);
            ui.close();
        }
        crate::house::menu_sep(ui);
        if row(ui, "Move to bin", "Delete", true) {
            out.push(Action::Delete(targets));
            self.selected.clear();
            ui.close();
        }
    }

    /// Keys while the window is open (taken from the grid by files.rs): Enter, F2, Delete,
    /// Backspace, Ctrl+A and the clipboard events.
    fn keys(&mut self, ui: &Ui, keys: &[egui::Event], rows: &[(Item, String, String, i64, Option<u64>)], out: &mut Vec<Action>) {
        if self.renaming.is_some() || ui.ctx().memory(|m| m.focused()) == Some(egui::Id::unique("starsheets_files_search")) {
            return;
        }
        for ev in keys {
            match ev {
                egui::Event::Copy if !self.selected.is_empty() => self.clipboard = Some((self.selected.clone(), false)),
                egui::Event::Cut if !self.selected.is_empty() => self.clipboard = Some((self.selected.clone(), true)),
                egui::Event::Paste(_) => self.paste_into(self.cwd.clone(), out),
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::Enter if self.selected.len() == 1 => {
                        let item = self.selected[0].clone();
                        self.activate(&item, false, out);
                    }
                    Key::F2 if self.selected.len() == 1 => {
                        let item = self.selected[0].clone();
                        self.start_rename(&item);
                    }
                    Key::Delete if !self.selected.is_empty() => {
                        out.push(Action::Delete(std::mem::take(&mut self.selected)));
                    }
                    Key::Backspace if !self.cwd.is_empty() => {
                        self.cwd = parent_of(&self.cwd).to_string();
                        self.selected.clear();
                    }
                    Key::A if modifiers.command => self.selected = rows.iter().map(|r| r.0.clone()).collect(),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Starts renaming a folder that was just made ("New folder").
    pub fn rename_new_folder(&mut self, path: &str) {
        self.selected = vec![Item::Folder(path.to_string())];
        self.renaming = Some((Item::Folder(path.to_string()), name_of(path).to_string(), true));
    }

    fn bin_ui(&mut self, ui: &mut Ui, out: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("‹  Back to my files").color(GREEN)).frame(false)).clicked() {
                self.in_bin = false;
                self.confirm_purge = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let sure = self.confirm_purge == Some(None);
                let label = if sure { "Click again to empty the bin" } else { "Empty bin" };
                if ui
                    .add_enabled(
                        !self.bin.is_empty(),
                        egui::Button::new(RichText::new(label).color(Color32::from_rgb(0xC5, 0x39, 0x29))).frame(false),
                    )
                    .clicked()
                {
                    if sure {
                        out.push(Action::Purge(None));
                        self.confirm_purge = None;
                    } else {
                        self.confirm_purge = Some(None);
                    }
                }
            });
        });
        ui.label(RichText::new("Files and folders moved to the bin stay here until you delete them forever.").color(theme::TEXT_DIM));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(400.0).auto_shrink([false, false]).show(ui, |ui| {
            if self.loading && self.bin.is_empty() {
                ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
            } else if self.bin.is_empty() {
                ui.label(RichText::new("The bin is empty.").color(theme::TEXT_DIM));
            }
            for b in self.bin.clone() {
                let (r, _) = ui.allocate_exact_size(vec2(W - 12.0, ROW), Sense::hover());
                let p = ui.painter().clone();
                if let Some(img) = crate::icons::icon(if b.folder { "folder_open" } else { "starsheets" }) {
                    img.paint_at(ui, egui::Rect::from_center_size(r.left_center() + vec2(20.0, 0.0), vec2(22.0, 22.0)));
                }
                p.text(r.left_center() + vec2(44.0, -7.0), Align2::LEFT_CENTER, name_of(&b.original), FontId::proportional(14.0), theme::TEXT);
                let from = if parent_of(&b.original).is_empty() { "My files".to_string() } else { parent_of(&b.original).to_string() };
                p.text(
                    r.left_center() + vec2(44.0, 9.0),
                    Align2::LEFT_CENTER,
                    format!("from {from} · deleted {}", crate::files::date_text(b.deleted)),
                    FontId::proportional(11.0),
                    theme::TEXT_DIM,
                );
                let restore = egui::Rect::from_min_size(r.right_top() + vec2(-250.0, 4.0), vec2(90.0, ROW - 8.0));
                let purge = egui::Rect::from_min_size(r.right_top() + vec2(-150.0, 4.0), vec2(140.0, ROW - 8.0));
                if ui.put(restore, egui::Button::new(RichText::new("Restore").color(GREEN))).clicked() {
                    out.push(Action::Restore(b.name.clone()));
                }
                let sure = self.confirm_purge.as_ref() == Some(&Some(b.name.clone()));
                if ui
                    .put(
                        purge,
                        egui::Button::new(
                            RichText::new(if sure { "Sure? Click again" } else { "Delete forever" }).color(Color32::from_rgb(0xC5, 0x39, 0x29)),
                        ),
                    )
                    .clicked()
                {
                    if sure {
                        out.push(Action::Purge(Some(b.name.clone())));
                        self.confirm_purge = None;
                    } else {
                        self.confirm_purge = Some(Some(b.name.clone()));
                    }
                }
            }
        });
    }
}
