//! Details › Versions: Drive's "Manage versions" for any file. The server keeps the earlier
//! versions of every file (`api/files/<id>/history`; only people who can edit the file may see
//! them). Each row has when, who and how big, with Download and "Restore this version" (a restore
//! is a new version, the old ones stay). "Upload new version" puts a picked file over this one.

use eframe::egui::{self, RichText};
use serde_json::Value as Json;

use crate::{house, theme};

/// What a click in the list asks for.
pub(super) enum Act {
    Download(String),
    Restore(String),
    Upload,
}

fn who(e: &Json) -> String {
    match e["author"].as_str().unwrap_or("") {
        "" => "someone".to_string(),
        a => a.to_string(),
    }
}

/// The rows as shown ("Current version · ann · 3 B"), newest first, for the test hook.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn rows(list: &Json) -> Vec<String> {
    let mut out = Vec::new();
    if list["current"].is_object() {
        out.push(format!("Current version · {} · {}", who(&list["current"]), super::size_text(list["current"]["size"].as_u64().unwrap_or(0))));
    }
    for e in list["versions"].as_array().map(Vec::as_slice).unwrap_or_default() {
        out.push(format!("{} · {} · {}", super::ago(e["saved"].as_i64().unwrap_or(0)), who(e), super::size_text(e["size"].as_u64().unwrap_or(0))));
    }
    out
}

/// The list with its buttons; `can_edit` is false for people who may only look (the server
/// wouldn't answer them anyway).
pub(super) fn ui(ui: &mut egui::Ui, list: Option<&Json>, can_edit: bool) -> Option<Act> {
    let mut act = None;
    if !can_edit {
        ui.label(RichText::new("Only people who can edit this file see its versions.").size(house::BODY).color(theme::TEXT_DIM));
        return None;
    }
    if ui.button("Upload new version…").clicked() {
        act = Some(Act::Upload);
    }
    house::target(ui.ctx(), "versions upload", ui.min_rect());
    ui.add_space(house::S);
    let Some(list) = list else {
        ui.label(RichText::new("Loading…").size(house::BODY).color(theme::TEXT_DIM));
        return act;
    };
    let shown = rows(list);
    let earlier = list["versions"].as_array().map(Vec::as_slice).unwrap_or_default();
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        for (i, text) in shown.iter().enumerate() {
            ui.horizontal(|ui| {
                let r = ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT));
                house::target(ui.ctx(), &format!("version row {i}"), r.rect);
                // Row 0 is the file as it is now; the others are earlier versions.
                if let Some(e) = i.checked_sub(1).and_then(|k| earlier.get(k)) {
                    let v = e["v"].as_str().unwrap_or("").to_string();
                    let d = ui.small_button("Download");
                    house::target(ui.ctx(), &format!("version download {i}"), d.rect);
                    if d.clicked() {
                        act = Some(Act::Download(v.clone()));
                    }
                    let b = ui.small_button("Restore this version");
                    house::target(ui.ctx(), &format!("version restore {i}"), b.rect);
                    if b.clicked() {
                        act = Some(Act::Restore(v));
                    }
                }
            });
            ui.add_space(house::XS);
        }
    });
    act
}
