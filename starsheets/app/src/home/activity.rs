//! Details › Activity: what happened to a file, newest first, as Google Drive lists it
//! ("Ada edited · 2 h ago"). The list comes from the server (`api/files/<id>/activity`), which
//! only answers people who can see the file.

use eframe::egui::{self, RichText};
use serde_json::Value as Json;

use crate::{house, theme};

fn now() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }
}

/// "just now", "5 min ago", "2 h ago", "3 days ago", then the date.
pub(super) fn relative(then: i64, now: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        86_400..=2_591_999 => {
            let d = s / 86_400;
            if d == 1 { "yesterday".into() } else { format!("{d} days ago") }
        }
        _ => super::ago(then),
    }
}

/// One entry as a sentence: "Ada renamed it to Budget".
pub(super) fn sentence(e: &Json) -> String {
    let who = e["name"].as_str().unwrap_or("Someone");
    let detail = e["detail"].as_str().unwrap_or("");
    let with = |what: &str| format!("{who} {what} {detail}");
    match e["kind"].as_str().unwrap_or("") {
        "created" if detail.is_empty() => format!("{who} created it"),
        "created" => format!("{who} created it as {detail}"),
        "edited" => format!("{who} edited"),
        "renamed" => with("renamed it to"),
        "moved" => with("moved it to"),
        "shared" => with("shared it with"),
        "unshared" => with("stopped sharing it with"),
        "link" => with("changed the link:"),
        "commented" => format!("{who} commented"),
        "restored" => format!("{who} restored an earlier version"),
        "trashed" => format!("{who} moved it to the bin"),
        other => format!("{who} {other}"),
    }
}

/// The rows as shown ("Ada edited · 2 h ago"), for the test hook.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn rows(list: &Json) -> Vec<String> {
    let now = now();
    list["activity"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|e| format!("{} · {}", sentence(e), relative(e["at"].as_i64().unwrap_or(0), now)))
        .collect()
}

/// The list, or a note while it loads.
pub(super) fn ui(ui: &mut egui::Ui, list: Option<&Json>) {
    let Some(list) = list else {
        ui.label(RichText::new("Loading…").size(house::BODY).color(theme::TEXT_DIM));
        return;
    };
    let entries = list["activity"].as_array().map(Vec::as_slice).unwrap_or_default();
    if entries.is_empty() {
        ui.label(RichText::new("No activity yet.").size(house::BODY).color(theme::TEXT_DIM));
        return;
    }
    let now = now();
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        for (i, e) in entries.iter().enumerate() {
            let text = format!("{} · {}", sentence(e), relative(e["at"].as_i64().unwrap_or(0), now));
            let r = ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT));
            house::target(ui.ctx(), &format!("activity row {i}"), r.rect);
            ui.add_space(house::XS);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_and_sentences_read_like_drive() {
        assert_eq!(relative(1000, 1030), "just now");
        assert_eq!(relative(1000, 1000 + 2 * 3600 + 5), "2 h ago");
        assert_eq!(relative(1000, 1000 + 5 * 60), "5 min ago");
        assert_eq!(relative(1000, 1000 + 86_400), "yesterday");
        let t = |kind: &str, detail: &str| sentence(&json!({"name": "Ada", "kind": kind, "detail": detail}));
        assert_eq!(t("edited", ""), "Ada edited");
        assert_eq!(t("renamed", "Budget"), "Ada renamed it to Budget");
        assert_eq!(t("shared", "ann (can edit)"), "Ada shared it with ann (can edit)");
        assert_eq!(t("restored", "v3"), "Ada restored an earlier version");
        assert_eq!(t("created", ""), "Ada created it");
    }
}
