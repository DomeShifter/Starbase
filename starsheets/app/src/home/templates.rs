//! "From a template…" (gaps list item 1): a gallery of starter files, grouped by app. Choosing
//! one asks the store for a copy in the current folder (`POST api/templates/<key>`, made like an
//! upload: "CV (copy).docx") and opens it. Opened from the Drive's New menu.

use super::*;
use starbase_templates::{ALL, Kind, Template};

/// Which folder the gallery shows for, while it is open (kept in egui's memory, so the Drive
/// needs no field for it).
#[derive(Clone)]
struct Open(String);

fn key() -> egui::Id {
    egui::Id::unique("templates_gallery")
}

/// Opens the gallery: the copy will be made in `folder`.
pub(super) fn open(ctx: &egui::Context, folder: &str) {
    ctx.data_mut(|d| d.insert_temp(key(), Open(folder.to_string())));
}

/// The groups the gallery shows, in order: each app's heading and its templates.
pub(super) fn groups() -> Vec<(&'static str, Vec<&'static Template>)> {
    [Kind::Document, Kind::Spreadsheet, Kind::Presentation]
        .into_iter()
        .map(|k| (k.heading(), ALL.iter().filter(|t| t.kind == k).collect::<Vec<_>>()))
        .filter(|(_, ts)| !ts.is_empty())
        .collect()
}

/// The request that makes a copy of `t` in `folder`.
pub(super) fn copy_url(t: &Template, folder: &str) -> String {
    format!("api/templates/{}?folder={}", t.key, enc(folder))
}

impl Home {
    /// Shows the gallery while it is open (each frame).
    pub(super) fn templates_ui(&mut self, ctx: &egui::Context) {
        let Some(Open(folder)) = ctx.data(|d| d.get_temp::<Open>(key())) else { return };
        let mut keep = true;
        let mut chosen = None;
        house::dialog(ctx, "templates_dialog", 620.0, "New from a template", Some(&mut keep), |ui| {
            for (heading, list) in groups() {
                ui.label(RichText::new(heading).strong().color(theme::TEXT));
                ui.horizontal_wrapped(|ui| {
                    for t in list {
                        if card(ui, t).clicked() {
                            chosen = Some(*t);
                        }
                    }
                });
                ui.add_space(house::S);
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            keep = false;
        }
        if let Some(t) = chosen {
            keep = false;
            if t.kind == Kind::Presentation && !self.store.has_slides() {
                self.error = Some("Presentations can't be made here.".into());
            } else {
                let store = self.store.clone();
                call(&self.store, &self.inbox, ctx, Method::Post, copy_url(&t, &folder), None, move |b| {
                    let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                    let (id, name) = (v["id"].as_str().unwrap_or(""), v["name"].as_str().unwrap_or(""));
                    match store.open(id, name) {
                        Ok(()) => Reply::Refresh,
                        Err(e) => Reply::Error(e),
                    }
                });
            }
        }
        if !keep {
            ctx.data_mut(|d| d.remove::<Open>(key()));
        }
    }
}

/// One template's card: its app's icon, the name and a line about it.
fn card(ui: &mut Ui, t: &Template) -> egui::Response {
    let r = house::card_frame()
        .show(ui, |ui| {
            ui.set_width(170.0);
            ui.set_height(110.0);
            ui.vertical(|ui| {
                if let Some(img) = crate::icons::icon(t.icon) {
                    ui.add(img.fit_to_exact_size(vec2(32.0, 32.0)));
                }
                ui.label(RichText::new(t.name).size(15.0).color(theme::TEXT));
                ui.label(RichText::new(t.blurb).small().color(theme::TEXT_DIM));
            });
        })
        .response;
    let r = ui.interact(r.rect, egui::Id::unique(("template", t.key)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t.name));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every template is in the gallery once, under its app's heading, with a copy request
    /// into the folder (table test over every choice).
    #[test]
    fn every_template_is_offered_once_and_copies_into_the_folder() {
        let g = groups();
        assert_eq!(g.iter().map(|(h, _)| *h).collect::<Vec<_>>(), ["Documents", "Spreadsheets", "Presentations"]);
        let all: Vec<&str> = g.iter().flat_map(|(_, ts)| ts.iter().map(|t| t.key)).collect();
        assert_eq!(all, ALL.iter().map(|t| t.key).collect::<Vec<_>>());
        for t in ALL {
            assert_eq!(copy_url(t, "me/Work plans"), format!("api/templates/{}?folder={}", t.key, enc("me/Work plans")));
            assert!(crate::icons::icon(t.icon).is_some(), "{}: the app ships its icon", t.key);
        }
    }
}
