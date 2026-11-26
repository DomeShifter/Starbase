//! Photo Mode's own room: while it is open the window is dark, the left Drive list and the
//! Drive's New/list/grid controls are gone, and the top bar holds only photo things: back to the
//! Drive, "Search photos", the SET switch (this folder / with subfolders / everything), the
//! Type filter and the sun/moon. Leaving puts the Drive back as it was (theme::sync_dark).

use egui::{RichText, Ui, vec2};

use super::super::Home;
use super::{PhotoMode, Pic};
use crate::{house, theme};

/// Which pictures every Photo Mode view works on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(in crate::home) enum Scope {
    /// The folder Photo Mode was opened from.
    #[default]
    Folder,
    /// That folder and every folder inside it.
    Sub,
    /// Every picture the person can see in the Drive.
    All,
}

impl Scope {
    const ALL: [Scope; 3] = [Scope::Folder, Scope::Sub, Scope::All];

    pub fn label(self) -> &'static str {
        match self {
            Scope::Folder => "This folder",
            Scope::Sub => "This folder and subfolders",
            Scope::All => "Everything",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Scope::Folder => "This folder",
            Scope::Sub => "With subfolders",
            Scope::All => "Everything",
        }
    }
}

/// The Type filter's choices, in the order shown.
pub(in crate::home) const KINDS: [&str; 7] = ["JPEG", "PNG", "HEIC", "RAW", "GIF", "WebP", "Video"];

/// Which of `KINDS` a picture is (by its media type, then its file extension).
pub(in crate::home) fn kind_of(p: &Pic) -> Option<usize> {
    let m = p.media.to_lowercase();
    let ext = p.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    Some(if m.starts_with("video/") {
        6
    } else if m == "image/x-raw" || matches!(ext.as_str(), "dng" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "raf" | "pef" | "srw") {
        3
    } else if m.contains("jpeg") || matches!(ext.as_str(), "jpg" | "jpeg") {
        0
    } else if m.contains("png") || ext == "png" {
        1
    } else if m.contains("heic") || m.contains("heif") || matches!(ext.as_str(), "heic" | "heif") {
        2
    } else if m.contains("gif") || ext == "gif" {
        4
    } else if m.contains("webp") || ext == "webp" {
        5
    } else {
        return None;
    })
}

pub(in crate::home) struct Shell {
    /// The dark look (the default in Photo Mode; the sun/moon switches it for this session).
    pub dark: bool,
    pub scope: Scope,
    /// The Type filter: which of `KINDS` are shown (none ticked = all).
    pub types: [bool; 7],
    /// The "Search photos" text.
    pub query: String,
    /// Everything the server listed; `PhotoMode::pics` is this, filtered.
    pub full: Vec<Pic>,
}

impl Default for Shell {
    fn default() -> Shell {
        Shell { dark: true, scope: Scope::Folder, types: [false; 7], query: String::new(), full: Vec::new() }
    }
}

impl Shell {
    /// Whether `p` passes the Type filter and the search (every word in its name, place, camera or date).
    pub fn matches(&self, p: &Pic) -> bool {
        if self.types.iter().any(|t| *t) && !kind_of(p).is_some_and(|k| self.types[k]) {
            return false;
        }
        let q = self.query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let hay = format!(
            "{} {} {} {} {}",
            p.name,
            p.place.as_deref().unwrap_or(""),
            p.camera.as_deref().unwrap_or(""),
            p.taken.as_deref().unwrap_or(""),
            super::date_label(p.taken.as_deref(), p.modified)
        )
        .to_lowercase();
        q.split_whitespace().all(|w| hay.contains(w))
    }

    pub fn filtered(&self) -> Vec<Pic> {
        self.full.iter().filter(|p| self.matches(p)).cloned().collect()
    }

    pub fn filtering(&self) -> bool {
        self.types.iter().any(|t| *t) || !self.query.trim().is_empty()
    }
}

impl PhotoMode {
    /// `pics` again from the full list, after the filter or the search changed.
    pub(in crate::home) fn refilter(&mut self) {
        self.pics = self.shell.filtered();
        let ids: std::collections::HashSet<&str> = self.pics.iter().map(|p| p.id.as_str()).collect();
        self.alb.selected.retain(|id| ids.contains(id.as_str()));
        if let Some(vw) = &mut self.viewer {
            vw.fit_list(self.pics.len());
        }
    }

    /// The search text, from the top bar (empty outside Photo Mode).
    pub(in crate::home) fn shell_query(&mut self, q: &str) {
        if self.shell.query != q {
            self.shell.query = q.to_string();
            self.refilter();
        }
    }

    /// Switches the set of pictures and lists again.
    pub(in crate::home) fn set_scope(&mut self, ctx: &egui::Context, store: &crate::store::Backend, scope: Scope) {
        if self.shell.scope == scope {
            return;
        }
        self.shell.scope = scope;
        self.sub = scope != Scope::Folder;
        self.pics.clear();
        self.shell.full.clear();
        if let Some(f) = self.folder.clone() {
            self.list(ctx, store, &f);
        }
    }
}

/// A rounded button for the top bar: an optional icon and a label.
pub(super) fn pill(ui: &mut Ui, icon: Option<&str>, label: &str, on: bool) -> egui::Response {
    let text = RichText::new(label).size(house::BODY).color(theme::text());
    let b = match icon.and_then(crate::icons::icon) {
        Some(img) if label.is_empty() => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
        Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(18.0, 18.0)), text),
        None => egui::Button::new(text),
    };
    ui.add(b.fill(if on { theme::share() } else { theme::menu_hover() }).corner_radius(16).min_size(vec2(36.0, 32.0)))
}

impl Home {
    /// The top bar in Photo Mode.
    pub(in crate::home) fn photo_top_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let narrow = ui.available_width() < 640.0;
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let back = pill(ui, Some("arrow_back"), "", false).on_hover_text("Back to Drive");
            house::target(&ctx, "photo-exit", back.rect);
            if back.clicked() {
                self.photos.set_on(&self.store, false);
                self.search.clear();
            }
            if let Some(img) = crate::icons::icon_in("photo_mode", theme::BLUE) {
                ui.add(img.fit_to_exact_size(vec2(28.0, 28.0)));
            }
            if !narrow {
                ui.label(RichText::new("Photo Mode").size(22.0).color(theme::text()));
                ui.add_space(house::M);
            }
            let reserve = if narrow { 130.0 } else { 520.0 };
            let w = (ui.available_width() - reserve).clamp(120.0, 700.0);
            let search = house::field(&mut self.search, house::SEARCH_TEXT)
                .id(egui::Id::unique("photo_search"))
                .hint_text(RichText::new("Search photos").size(house::SEARCH_TEXT))
                .frame(egui::Frame::NONE)
                .margin(vec2(0.0, 0.0))
                .min_size(vec2(0.0, 24.0))
                .desired_width((w - 64.0).max(60.0));
            let boxed = ui.allocate_ui_with_layout(vec2(w, 48.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                egui::Frame::NONE
                    .fill(if theme::photo_dark() { theme::menu_hover() } else { theme::toolbar() })
                    .corner_radius(24)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        if let Some(img) = crate::icons::icon("search") {
                            ui.add(img.fit_to_exact_size(vec2(20.0, 20.0)));
                        }
                        ui.add_space(house::S);
                        ui.add(search);
                    });
            });
            house::target(&ctx, "photo-search", boxed.response.rect);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                #[cfg(target_arch = "wasm32")]
                crate::account::me_ui(ui);
                #[cfg(not(target_arch = "wasm32"))]
                self.connect.menu_ui(ui, &self.store);
                if narrow {
                    let b = pill(ui, Some("filter_list"), "", self.photos.shell.filtering()).on_hover_text("Set, type and theme");
                    house::target(&ctx, "photo-menu", b.rect);
                    egui::Popup::menu(&b).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        house::menu(ui, 280.0);
                        house::caps(ui, "Set");
                        self.scope_rows(ui, &ctx);
                        house::menu_sep(ui);
                        house::caps(ui, "Type");
                        self.type_rows(ui);
                        house::menu_sep(ui);
                        self.theme_row(ui);
                        house::menu_sep(ui);
                        ui.horizontal_wrapped(|ui| self.photos.alb.ppl.scan.ui(ui, &self.store));
                    });
                } else {
                    let dark = self.photos.shell.dark;
                    let t = pill(ui, Some(if dark { "light_mode" } else { "dark_mode" }), "", false).on_hover_text(if dark {
                        "Light theme"
                    } else {
                        "Dark theme"
                    });
                    house::target(&ctx, "photo-theme", t.rect);
                    if t.clicked() {
                        self.photos.shell.dark = !dark;
                    }
                    let n = self.photos.shell.types.iter().filter(|t| **t).count();
                    let label = match n {
                        0 => "All types".to_string(),
                        1 => KINDS[self.photos.shell.types.iter().position(|t| *t).unwrap_or(0)].to_string(),
                        n => format!("{n} types"),
                    };
                    let ty = pill(ui, Some("filter_list"), &label, n > 0);
                    house::target(&ctx, "photo-type", ty.rect);
                    egui::Popup::menu(&ty).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        house::menu(ui, 200.0);
                        self.type_rows(ui);
                    });
                    self.photos.alb.ppl.scan.ui(ui, &self.store);
                    let set = pill(ui, Some("folder"), self.photos.shell.scope.short(), self.photos.shell.scope != Scope::Folder);
                    house::target(&ctx, "photo-set", set.rect);
                    egui::Popup::menu(&set).show(|ui| {
                        house::menu(ui, 280.0);
                        self.scope_rows(ui, &ctx);
                    });
                }
            });
        });
    }

    fn scope_rows(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        for s in Scope::ALL {
            let on = self.photos.shell.scope == s;
            if house::menu_row(ui, Some(if on { "check" } else { "" }), s.label(), "", true) {
                self.photos.set_scope(ctx, &self.store, s);
            }
        }
    }

    fn type_rows(&mut self, ui: &mut Ui) {
        if house::menu_row(ui, Some(if self.photos.shell.filtering_types() { "" } else { "check" }), "All types", "", true) {
            self.photos.shell.types = [false; 7];
            self.photos.refilter();
        }
        for (i, k) in KINDS.iter().enumerate() {
            let on = self.photos.shell.types[i];
            if house::menu_row(ui, Some(if on { "check" } else { "" }), k, "", true) {
                self.photos.shell.types[i] = !on;
                self.photos.refilter();
            }
        }
    }

    fn theme_row(&mut self, ui: &mut Ui) {
        let dark = self.photos.shell.dark;
        if house::menu_row(ui, Some(if dark { "light_mode" } else { "dark_mode" }), if dark { "Light theme" } else { "Dark theme" }, "", true) {
            self.photos.shell.dark = !dark;
        }
    }

    /// The middle of the window in Photo Mode: only the pictures.
    pub(in crate::home) fn photo_main_ui(&mut self, ui: &mut Ui, folder: &str) {
        if let Some(e) = self.error.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                if ui.add(house::link("OK")).clicked() {
                    self.error = None;
                }
            });
        }
        self.photos.ui(ui, &self.store, folder);
        self.photo_outs(ui.ctx());
    }
}

impl Shell {
    fn filtering_types(&self) -> bool {
        self.types.iter().any(|t| *t)
    }
}
