//! The Keyboard shortcuts dialog, as Google Docs' Help › Keyboard shortcuts (Ctrl+/): every menu
//! command that has a shortcut, grouped by its menu, plus the usual text-editing keys that no menu
//! lists. A search box filters by name or by keys.

use egui::{Align, Layout, RichText, Ui, vec2};

use crate::shell::Item;
use crate::theme::palette;

/// Keys that work while typing but have no menu entry.
const TEXT_EDITING: &[(&str, &str)] = &[
    ("Delete the word before the cursor", "Ctrl+Backspace"),
    ("Delete the word after the cursor", "Ctrl+Delete"),
    ("Move one word left", "Ctrl+Left"),
    ("Move one word right", "Ctrl+Right"),
    ("Start of the line", "Home"),
    ("End of the line", "End"),
    ("Start of the document", "Ctrl+Home"),
    ("End of the document", "Ctrl+End"),
    ("Select a character or line at a time", "Shift+Arrow keys"),
    ("Select a word at a time", "Ctrl+Shift+Left/Right"),
    ("Select to the start of the line", "Shift+Home"),
    ("Select to the end of the line", "Shift+End"),
    ("Select to the start of the document", "Ctrl+Shift+Home"),
    ("Select to the end of the document", "Ctrl+Shift+End"),
    ("New line in the same paragraph", "Shift+Enter"),
];

#[derive(Default)]
pub struct Shortcuts {
    pub open: bool,
    filter: String,
    /// Whether the dialog was drawn last frame, so opening it clears and focuses the search box.
    shown: bool,
}

/// Every command with a shortcut under `items`, as (label, keys); a command inside a sub-menu is
/// named "Sub-menu › Command" so "Left" reads as "Align & indent › Left".
fn collect(items: &[Item], path: &str, out: &mut Vec<(String, &'static str)>) {
    for item in items {
        match item {
            Item::Cmd(label, _, _, keys) | Item::Toggle(label, _, _, keys) if !keys.is_empty() => {
                let name = if path.is_empty() { (*label).to_string() } else { format!("{path} › {label}") };
                if !out.iter().any(|(n, k)| *n == name && k == keys) {
                    out.push((name, keys));
                }
            }
            Item::Sub(label, sub) => {
                let inner = if path.is_empty() { (*label).to_string() } else { format!("{path} › {label}") };
                collect(sub, &inner, out);
            }
            _ => {}
        }
    }
}

/// "Ctrl+Shift+V" as its keys; a "+" key itself ("Ctrl++") stays one key.
fn keys_of(keys: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = keys;
    while !rest.is_empty() {
        // A '+' at the start of what's left is the key, not a separator.
        let cut = rest.char_indices().skip(1).find(|&(_, c)| c == '+').map_or(rest.len(), |(i, _)| i);
        out.push(&rest[..cut]);
        rest = rest.get(cut + 1..).unwrap_or("");
    }
    out
}

/// How the keys read: "Ctrl + Shift + V", plain grey text as Google lists them.
fn spaced(keys: &str) -> String {
    keys_of(keys).join(" + ")
}

/// One shortcut: what it does at the left, its keys in grey at the right, a 36 px row.
fn row(ui: &mut Ui, label: &str, keys: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.label(RichText::new(label).size(14.0).color(palette().text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(spaced(keys)).size(14.0).color(palette().text_dim));
        });
    });
}

impl Shortcuts {
    /// Draws the dialog when open. `menus` is shell::MENUS.
    pub fn ui(&mut self, ctx: &egui::Context, menus: &[(&str, &[Item])]) {
        if !self.open {
            self.shown = false;
            return;
        }
        let opening = !self.shown;
        self.shown = true;
        if opening {
            self.filter.clear();
        }
        let mut groups: Vec<(&str, Vec<(String, &str)>)> = menus
            .iter()
            .map(|(menu, items)| {
                let mut list = Vec::new();
                collect(items, "", &mut list);
                (*menu, list)
            })
            .collect();
        groups.push(("Text editing", TEXT_EDITING.iter().map(|(l, k)| ((*l).to_string(), *k)).collect()));
        let needle = self.filter.trim().to_lowercase();
        // "ctrl+b", "ctrl b" and "ctrlb" all find Ctrl+B.
        let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace() && *c != '+').collect::<String>().to_lowercase();
        let needle_keys = squash(&needle);
        for (_, list) in &mut groups {
            list.retain(|(l, k)| {
                needle.is_empty() || l.to_lowercase().contains(&needle) || (!needle_keys.is_empty() && squash(k).contains(&needle_keys))
            });
        }

        let mut close = false;
        // Google's dialog: most of the window's height, the list scrolling inside it.
        let screen = ctx.content_rect();
        let list_height = (screen.height() * 0.8 - 150.0).max(160.0);
        let width = (screen.width() - 48.0).clamp(320.0, 720.0);
        let modal = egui::Modal::new(egui::Id::unique("stardocs_shortcuts")).frame(crate::looks::dialog_frame());
        let shown = modal.show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                crate::looks::title(ui, "Keyboard shortcuts");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let x = match crate::icons::icon_px("close", 24.0) {
                        Some(img) => egui::Button::image(img),
                        None => egui::Button::new(RichText::new("×").size(20.0).color(palette().text_dim)),
                    };
                    let r = ui.add(x.frame(false).min_size(vec2(40.0, 40.0)));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
                    if r.on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(12.0);
            let search = ui
                .scope(|ui| {
                    crate::looks::field_style(ui);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("Search shortcuts")
                            .desired_width(f32::INFINITY)
                            .text_color(palette().text),
                    )
                })
                .inner;
            if opening {
                search.request_focus();
            }
            ui.add_space(8.0);
            // A fixed height, so the dialog keeps its size while the list is filtered.
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(list_height).min_scrolled_height(list_height).show(ui, |ui| {
                let mut any = false;
                for (menu, list) in &groups {
                    if list.is_empty() {
                        continue;
                    }
                    ui.add_space(if any { 16.0 } else { 4.0 });
                    any = true;
                    ui.label(RichText::new(*menu).font(wordcraft_ui_egui::theme::medium(16.0)).color(palette().text));
                    ui.add_space(4.0);
                    for (label, keys) in list {
                        row(ui, label, keys);
                    }
                }
                if !any {
                    ui.add_space(12.0);
                    ui.label(RichText::new("No shortcuts match your search.").size(14.0).color(palette().text_dim));
                }
            });
        });
        if close || shown.should_close() {
            self.open = false;
            self.shown = false;
        }
    }
}
