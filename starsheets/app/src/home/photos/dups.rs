//! Photos > Duplicates: pictures with exactly the same content, side by side in groups. "Keep this
//! one" moves the others to the bin (the Drive's own bin, so Undo works). The server side is
//! `GET /api/photos/duplicates` (crates/server/src/photos/routes.rs).

use egui::{Color32, RichText, Sense, Ui, vec2};
use serde_json::Value as Json;

use super::albums::get;
use super::{Got, Out, PhotoMode, Pic, SMALL, cover_uv, date_label};
use crate::house;
use crate::store::{Backend, enc};
use crate::theme;

/// One group: pictures with the same content, each with the folder it lives in.
#[derive(Clone, Debug, Default)]
pub(in crate::home) struct Group {
    pub pics: Vec<(Pic, String)>,
}

#[derive(Default)]
pub(in crate::home) struct Dups {
    /// The folder the groups were asked for (None: not asked yet).
    pub asked: Option<String>,
    pub loaded: bool,
    pub enabled: bool,
    pub wasted: u64,
    pub groups: Vec<Group>,
}

pub(super) fn human(b: u64) -> String {
    let (b, units) = (b as f64, ["B", "KB", "MB", "GB", "TB"]);
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{} B", b as u64) } else { format!("{v:.1} {}", units[i]) }
}

impl Dups {
    pub fn take(&mut self, folder: &str, v: &Json) {
        if self.asked.as_deref() != Some(folder) {
            return;
        }
        self.loaded = true;
        self.enabled = v["enabled"].as_bool().unwrap_or(false);
        self.groups = v["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| Group {
                pics: PhotoMode::parse_pics(g)
                    .into_iter()
                    .zip(g["photos"].as_array().into_iter().flatten().map(|p| p["folder"].as_str().unwrap_or("").to_string()))
                    .collect(),
            })
            .collect();
        self.recount();
    }

    fn recount(&mut self) {
        self.wasted = self.groups.iter().map(|g| g.pics.iter().skip(1).map(|(p, _)| p.size).sum::<u64>()).sum();
    }

    pub fn reset(&mut self) {
        *self = Dups::default();
    }

    /// Keeps picture `index` of group `group`: the others are returned (to be binned) and the group goes.
    pub fn keep(&mut self, group: usize, index: usize) -> Option<Vec<String>> {
        let g = self.groups.get(group)?;
        g.pics.get(index)?;
        let ids = g.pics.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, (p, _))| p.id.clone()).collect();
        self.groups.remove(group);
        self.recount();
        Some(ids)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        serde_json::json!({
            "loaded": self.loaded, "enabled": self.enabled, "wasted": self.wasted,
            "groups": self.groups.iter().map(|g| serde_json::json!({
                "photos": g.pics.iter().map(|(p, f)| serde_json::json!({"id": p.id, "name": p.name, "folder": f, "size": p.size})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

impl PhotoMode {
    /// The Duplicates tab for `folder` (and everything inside it).
    pub(super) fn dups_ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        let ctx = ui.ctx().clone();
        if self.dups.asked.as_deref() != Some(folder) {
            self.dups.reset();
            self.dups.asked = Some(folder.to_string());
            let f = folder.to_string();
            get(&ctx, store, &self.inbox, &format!("api/photos/duplicates?folder={}", enc(folder)), move |v| Got::Dups(f, v));
        }
        ui.add_space(house::S);
        let dim = |t: String| RichText::new(t).size(house::BODY).color(theme::text_dim());
        if !self.dups.loaded {
            ui.label(dim("Looking for duplicates…".into()));
            return;
        }
        if !self.dups.enabled {
            ui.label(dim("Turn Photos on for this folder first (Photos tab).".into()));
            return;
        }
        if self.dups.groups.is_empty() {
            ui.label(dim("No duplicates: no two pictures here have exactly the same content.".into()));
            return;
        }
        let n = self.dups.groups.len();
        ui.label(
            RichText::new(format!(
                "{n} {} of identical pictures; keeping one of each frees {}",
                if n == 1 { "group" } else { "groups" },
                human(self.dups.wasted)
            ))
            .size(house::BODY)
            .color(theme::text()),
        );
        ui.add_space(house::S);
        let mut keep = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (gi, g) in self.dups.groups.clone().iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = vec2(house::L, house::L);
                    for (pi, (p, folder)) in g.pics.iter().enumerate() {
                        ui.vertical(|ui| {
                            let side = 160.0;
                            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                            let painter = ui.painter_at(rect);
                            painter.rect_filled(rect, 8, Color32::from_rgb(0xE8, 0xEA, 0xED));
                            self.want_id(&ctx, store, &p.id, p.v.as_deref(), SMALL);
                            let frame = self.frame;
                            if let Some(t) = self.tex.get(&(p.id.clone(), SMALL), frame) {
                                let [w, h] = t.size();
                                painter.image(t.id(), rect, cover_uv(w as f32, h as f32), Color32::WHITE);
                            }
                            let path = if folder.is_empty() { p.name.clone() } else { format!("{folder}/{}", p.name) };
                            ui.set_max_width(side);
                            ui.label(RichText::new(path).size(house::NOTE).color(theme::text())).on_hover_text(&p.name);
                            ui.label(
                                RichText::new(format!("{} · {}", human(p.size), date_label(p.taken.as_deref(), p.modified)))
                                    .size(house::NOTE)
                                    .color(theme::text_dim()),
                            );
                            if ui.add(house::link("Keep this one")).clicked() {
                                keep = Some((gi, pi));
                            }
                        });
                    }
                });
                ui.separator();
            }
        });
        if let Some((g, i)) = keep {
            self.keep_dup(g, i);
        }
    }

    /// Keeps one picture of a group: the others go to the bin (with the Drive's Undo note).
    pub(super) fn keep_dup(&mut self, group: usize, index: usize) {
        if let Some(ids) = self.dups.keep(group, index) {
            self.out.push(Out::Delete(ids));
        }
    }
}
