//! The Keyboard shortcuts dialog, as Google Docs' Help › Keyboard shortcuts (Ctrl+/): every menu
//! command that has a shortcut, grouped by its menu, plus the usual text-editing keys that no menu
//! lists. A search box filters by name or by keys.

use egui::{Align, Color32, Layout, RichText, Ui, vec2};

use crate::shell::{DIVIDER, Item, PAGE, TEXT, TEXT_DIM};

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
            Item::Cmd(label, _, _, keys) if !keys.is_empty() => {
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

/// One key in a small grey rounded box.
fn key_cap(ui: &mut Ui, key: &str) {
    egui::Frame::new().fill(PAGE).stroke(egui::Stroke::new(1.0, DIVIDER)).corner_radius(4.0).inner_margin(vec2(6.0, 2.0)).show(ui, |ui| {
        ui.label(RichText::new(key).size(12.0).monospace().color(TEXT_DIM));
    });
}

fn row(ui: &mut Ui, label: &str, keys: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(RichText::new(label).size(14.0).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Right to left: the last key is drawn first.
            for (i, k) in keys_of(keys).iter().rev().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("+").size(12.0).color(TEXT_DIM));
                }
                key_cap(ui, k);
            }
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
        let modal = egui::Modal::new(egui::Id::unique("stardocs_shortcuts"))
            .frame(egui::Frame::popup(&ctx.global_style()).fill(Color32::WHITE).corner_radius(8.0).inner_margin(24.0));
        let shown = modal.show(ctx, |ui| {
            ui.set_width(592.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Keyboard shortcuts").size(22.0).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let x = egui::Button::new(RichText::new("×").size(20.0).color(TEXT_DIM)).frame(false).min_size(vec2(32.0, 32.0));
                    if ui.add(x).on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(12.0);
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Search shortcuts")
                    .desired_width(f32::INFINITY)
                    .margin(vec2(8.0, 6.0))
                    .text_color(TEXT),
            );
            if opening {
                search.request_focus();
            }
            ui.add_space(8.0);
            // A fixed height, so the dialog keeps its size while the list is filtered.
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(380.0).show(ui, |ui| {
                let mut any = false;
                for (menu, list) in &groups {
                    if list.is_empty() {
                        continue;
                    }
                    if any {
                        ui.add_space(8.0);
                    }
                    any = true;
                    ui.label(RichText::new(*menu).size(14.0).strong().color(TEXT));
                    ui.add_space(2.0);
                    ui.separator();
                    for (label, keys) in list {
                        row(ui, label, keys);
                    }
                }
                if !any {
                    ui.add_space(12.0);
                    ui.label(RichText::new("No shortcuts match your search.").size(14.0).color(TEXT_DIM));
                }
            });
        });
        if close || shown.should_close() {
            self.open = false;
            self.shown = false;
        }
    }
}
