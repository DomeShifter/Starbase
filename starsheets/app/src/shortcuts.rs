//! Help › Keyboard shortcuts (Ctrl+/) and Help › Search the menus (Alt+/), as in Google Sheets.
//! Both read the menu table (`shell::MENUS`), so a new menu entry shows up in them by itself.
//! The shortcuts window mirrors StarDocs' (stardocs/app/src/shortcuts.rs): a search box, then
//! every shortcut grouped by menu under bold headings, keys as plain grey text ("Ctrl + B") in a
//! list about 80% of the window tall.

use egui::{Align, Layout, RichText, Ui, vec2};
use gridcraft_ui_egui::SheetApp;

use crate::shell::{Entry, entries};
use crate::{house, theme};

/// Ask for the windows (the menus and keys set these in egui's memory).
pub const SHORTCUTS_REQUEST: &str = "starsheets_shortcuts";
pub const SEARCH_REQUEST: &str = "starsheets_menu_search";

/// Keys that work on the grid but have no menu entry.
const GRID_KEYS: &[(&str, &str)] = &[
    ("Select all", "Ctrl+A"),
    ("Select the column", "Ctrl+Space"),
    ("Select the row", "Shift+Space"),
    ("Edit the active cell", "F2"),
    ("Fill down", "Ctrl+D"),
    ("Fill right", "Ctrl+R"),
    ("Fill the selection with what you typed", "Ctrl+Enter"),
    ("Insert today's date", "Ctrl+;"),
    ("Insert the time", "Ctrl+Shift+;"),
    ("Go to the start of the sheet", "Ctrl+Home"),
    ("Go to the last cell with data", "Ctrl+End"),
    ("Find", "Ctrl+F"),
    ("Hide the menus", "Ctrl+Shift+F"),
];

const WIDTH: f32 = 592.0;

/// Whether a menu's right-hand text is a key combination (Number formats show examples there).
pub fn is_keys(keys: &str) -> bool {
    ["Ctrl+", "Alt+", "Shift+", "Delete"].iter().any(|p| keys.starts_with(p))
        || (keys.starts_with('F') && keys[1..].chars().all(|c| c.is_ascii_digit()) && keys.len() > 1)
}

/// "Ctrl+Shift+V" as its keys; a "+" key itself ("Ctrl++") stays one key.
fn keys_of(keys: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = keys;
    while !rest.is_empty() {
        let cut = rest.char_indices().skip(1).find(|&(_, c)| c == '+').map_or(rest.len(), |(i, _)| i);
        out.push(&rest[..cut]);
        rest = rest.get(cut + 1..).unwrap_or("");
    }
    out
}

/// Keys as Google writes them: plain grey text, "Ctrl + Shift + V".
fn keys_text(keys: &str) -> String {
    keys_of(keys).join(" + ")
}

/// One shortcut: its name at the left, the keys in grey at the right, a 32 px row.
fn key_row(ui: &mut Ui, label: &str, keys: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), house::ROW_HEIGHT), egui::Sense::hover());
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.label(RichText::new(label).size(house::BODY).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(house::S);
            ui.label(RichText::new(keys_text(keys)).size(house::BODY).color(theme::TEXT_DIM));
        });
    });
}

/// The shortcut list, grouped by menu: (group, [(name, keys)]), filtered by `filter` (a name, or
/// keys typed as "ctrl+b", "ctrl b" or "ctrlb").
pub fn groups(filter: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for e in entries() {
        if !is_keys(e.keys) {
            continue;
        }
        let menu = e.path.split(" › ").next().unwrap_or_default().to_string();
        let name = e.path.split_once(" › ").map_or(e.path.clone(), |(_, rest)| rest.to_string());
        match out.iter_mut().find(|(m, _)| *m == menu) {
            Some((_, list)) if !list.iter().any(|(n, k)| *n == name && k == e.keys) => list.push((name, e.keys.to_string())),
            Some(_) => {}
            None => out.push((menu, vec![(name, e.keys.to_string())])),
        }
    }
    out.push(("Grid".into(), GRID_KEYS.iter().map(|(l, k)| ((*l).to_string(), (*k).to_string())).collect()));
    let needle = filter.trim().to_lowercase();
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace() && *c != '+').collect::<String>().to_lowercase();
    let needle_keys = squash(&needle);
    for (_, list) in &mut out {
        list.retain(|(l, k)| {
            needle.is_empty() || l.to_lowercase().contains(&needle) || (!needle_keys.is_empty() && squash(k).contains(&needle_keys))
        });
    }
    out.retain(|(_, l)| !l.is_empty());
    out
}

/// Menu entries whose name (with its menu path) holds every word of `query`.
pub fn search(query: &str) -> Vec<Entry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    entries().into_iter().filter(|e| words.iter().all(|w| e.path.to_lowercase().contains(w.as_str()))).collect()
}

#[derive(Default)]
pub struct Help {
    shortcuts: Option<String>,
    search: Option<String>,
    /// The highlighted search result (arrow keys move it).
    pick: usize,
    focus: bool,
}

impl Help {
    /// Draws whichever window is open; a picked search result comes back to run.
    pub fn ui(&mut self, app: &SheetApp, ctx: &egui::Context) -> Option<Entry> {
        for (key, slot) in [(SHORTCUTS_REQUEST, &mut self.shortcuts), (SEARCH_REQUEST, &mut self.search)] {
            if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(key))).is_some() {
                *slot = Some(String::new());
                self.focus = true;
                self.pick = 0;
            }
        }
        if self.shortcuts.is_some() {
            self.shortcuts_ui(ctx);
            return None;
        }
        if self.search.is_some() {
            return self.search_ui(app, ctx);
        }
        None
    }

    fn shortcuts_ui(&mut self, ctx: &egui::Context) {
        let Some(filter) = self.shortcuts.as_mut() else { return };
        let mut closed = false;
        let mut quit = false;
        let focus = std::mem::take(&mut self.focus);
        house::dialog(ctx, "starsheets_shortcuts", WIDTH, "Keyboard shortcuts", Some(&mut closed), |ui| {
            let r = ui.add(house::field(filter, house::SEARCH_TEXT).hint_text("Search shortcuts").desired_width(WIDTH));
            if focus {
                r.request_focus();
            }
            ui.add_space(house::S);
            let list = groups(filter);
            // A tall list, as in Sheets: about 80% of the window, less the title, search box and
            // the dialog's padding, so the last row is never cut off.
            let height = (ctx.content_rect().height() * 0.8 - 140.0).max(160.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(height).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, (menu, rows)) in list.iter().enumerate() {
                    ui.add_space(if i > 0 { house::L } else { house::XS });
                    ui.label(RichText::new(menu).font(gridcraft_ui_egui::theme::ui_bold(house::BODY + 2.0)).color(theme::TEXT));
                    ui.add_space(house::XS);
                    for (label, keys) in rows {
                        key_row(ui, label, keys);
                    }
                }
                ui.add_space(house::S);
                if list.is_empty() {
                    ui.add_space(house::M);
                    house::note(ui, "No shortcuts match your search.");
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                quit = true;
            }
        });
        if closed || quit {
            self.shortcuts = None;
        }
    }

    fn search_ui(&mut self, app: &SheetApp, ctx: &egui::Context) -> Option<Entry> {
        let query = self.search.as_mut()?;
        let mut closed = false;
        let mut quit = false;
        let mut chosen = None;
        let focus = std::mem::take(&mut self.focus);
        let pick = &mut self.pick;
        house::dialog(ctx, "starsheets_menu_search", 480.0, "Search the menus", Some(&mut closed), |ui| {
            let r = ui.add(house::field(query, house::SEARCH_TEXT).hint_text("Search the menus (Alt+/)").desired_width(480.0));
            if focus || (r.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))) {
                r.request_focus();
            }
            if r.changed() {
                *pick = 0;
            }
            let found = search(query);
            let n = found.len().min(12);
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) && n > 0 {
                    *pick = (*pick + 1) % n;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) && n > 0 {
                    *pick = (*pick + n - 1) % n;
                }
            });
            ui.add_space(house::S);
            crate::shell::menu_style_width(ui, 480.0);
            for (i, e) in found.iter().take(12).enumerate() {
                let on = crate::shell::entry_enabled(app, e.id);
                let label = crate::menuactions::label(&app.session, &e.path);
                let keys = if is_keys(e.keys) { e.keys } else { "" };
                let b = egui::Button::new(RichText::new(label).size(house::BODY).color(theme::TEXT))
                    .shortcut_text(RichText::new(keys).size(house::NOTE).color(theme::TEXT_DIM))
                    .fill(if i == *pick { theme::MENU_HOVER } else { egui::Color32::TRANSPARENT })
                    .frame_when_inactive(i == *pick)
                    .min_size(vec2(480.0, house::ROW_HEIGHT));
                if ui.add_enabled(on, b).clicked() {
                    chosen = Some(e.clone());
                }
            }
            if !query.trim().is_empty() && found.is_empty() {
                house::note(ui, "No menu items match your search.");
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(e) = found.get(*pick)
                && crate::shell::entry_enabled(app, e.id)
            {
                chosen = Some(e.clone());
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                quit = true;
            }
        });
        if closed || quit || chosen.is_some() {
            self.search = None;
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_list_holds_menu_keys_and_filters() {
        let all = groups("");
        let edit = all.iter().find(|(m, _)| m == "Edit").expect("an Edit group");
        assert!(edit.1.iter().any(|(n, k)| n == "Paste special › Values only" && k == "Ctrl+Shift+V"), "{edit:?}");
        let help = all.iter().find(|(m, _)| m == "Help").expect("a Help group");
        assert!(help.1.iter().any(|(_, k)| k == "Ctrl+/"));
        // Number formats show examples on the right ("1,000.12"), not keys.
        assert!(all.iter().all(|(_, l)| l.iter().all(|(_, k)| is_keys(k))));
        let bold = groups("ctrl b");
        assert!(bold.iter().any(|(_, l)| l.iter().any(|(n, _)| n.ends_with("Bold"))), "{bold:?}");
        assert!(groups("no such shortcut xyz").is_empty());
    }

    #[test]
    fn menu_search_finds_entries_by_any_words() {
        let hits = search("paste values");
        assert!(hits.iter().any(|e| e.path == "Edit › Paste special › Values only"), "{:?}", hits.iter().map(|e| &e.path).collect::<Vec<_>>());
        assert!(search("freeze 2 rows").iter().any(|e| e.id == "panel:freeze"));
        assert!(search("").is_empty());
    }

    #[test]
    fn keys_are_told_apart_from_examples() {
        for k in ["Ctrl+Shift+V", "Alt+/", "F2", "Shift+F11", "Delete"] {
            assert!(is_keys(k), "{k}");
        }
        for k in ["1,000.12", "£1,000", "", "Fig"] {
            assert!(!is_keys(k), "{k}");
        }
        assert_eq!(keys_of("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(keys_text("Ctrl+Shift+V"), "Ctrl + Shift + V");
        assert_eq!(keys_text("F2"), "F2");
    }
}
