//! Photo Mode's own room: while it is open the window is dark, the left Drive list and the
//! Drive's New/list/grid controls are gone, and the top bar holds only photo things: back to the
//! Drive, "Search photos", the SET switch (this folder / with subfolders / everything), the
//! Type filter and the sun/moon. Leaving puts the Drive back as it was (theme::sync_dark).

use egui::{RichText, Ui, vec2};

use super::super::Home;
use super::albums::Tab;
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

/// One part of the View menu (and, in a narrow window, of the one overflow menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewItem {
    /// A small-caps section heading.
    Head(&'static str),
    Sep,
    /// The SET rows (this folder / with subfolders / everything).
    Set,
    /// The Type rows (All types, then each kind).
    Types,
    /// Newest first / Oldest first.
    Sort,
    /// Dark theme / Light theme.
    Theme,
    /// Narrow window only: the toolbar's buttons that no longer fit.
    Upload,
    Slideshow,
    Scan,
}

/// The View menu, top to bottom: Show (set), Type, Sort, Theme. In a narrow window the toolbar's
/// Upload, Slideshow and Scan now join it (when they apply), as one overflow menu.
pub(super) fn view_items(narrow: bool, grid: bool, slideshow: bool) -> Vec<ViewItem> {
    use ViewItem::*;
    let mut v = vec![Head("Show"), Set, Sep, Head("Type"), Types];
    if grid {
        v.extend([Sep, Head("Sort"), Sort]);
    }
    v.extend([Sep, Head("Theme"), Theme]);
    if narrow {
        v.push(Sep);
        if grid {
            v.push(Upload);
        }
        if slideshow {
            v.push(Slideshow);
        }
        v.push(Scan);
    }
    v
}

impl Home {
    /// The top bar in Photo Mode: back, name, search; on the right Upload, View ▾, Scan now and
    /// Slideshow (in a narrow window all but the account sit in one overflow menu).
    pub(in crate::home) fn photo_top_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let narrow = ui.available_width() < 640.0;
        let folder = self.photo_folder();
        let pm = &self.photos;
        // Upload, sorting and Slideshow belong to the picture grid (Photos and Timeline).
        let grid = folder.is_some() && pm.enabled == Some(true) && matches!(pm.alb.tab, Tab::Photos | Tab::Timeline);
        let slideshow = grid && !pm.pics.is_empty();
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let back = house::icon_button(ui, "arrow_back", "Back to Drive", false);
            house::target(&ctx, "photo-exit", back.rect);
            if back.clicked() {
                self.photos.set_on(&self.store, false);
                self.search.clear();
            }
            if let Some(img) = crate::icons::photos_logo(32.0) {
                let logo = ui.add(img);
                house::target(&ctx, "photo-logo", logo.rect);
            }
            if !narrow {
                ui.label(RichText::new("Photo Mode").size(house::TITLE).color(theme::text()));
                ui.add_space(house::M);
            }
            let reserve = if narrow { 130.0 } else { 640.0 };
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
                ui.spacing_mut().item_spacing.x = house::S;
                #[cfg(target_arch = "wasm32")]
                crate::account::me_ui(ui);
                #[cfg(not(target_arch = "wasm32"))]
                self.connect.menu_ui(ui, &self.store);
                let lit = self.photos.shell.filtering() || self.photos.shell.scope != Scope::Folder;
                let items = view_items(narrow, grid, slideshow);
                if narrow {
                    let b = house::icon_button(ui, "filter_list", "View and more", lit);
                    house::target(&ctx, "photo-menu", b.rect);
                    egui::Popup::menu(&b).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        house::menu(ui, 280.0);
                        self.view_menu_ui(ui, &ctx, &items, folder.as_deref());
                    });
                    return;
                }
                if slideshow && ui.add(house::secondary_in("Slideshow", theme::BLUE)).clicked() {
                    self.photos.slideshow(&ctx);
                }
                self.photos.alb.ppl.scan.ui(ui, &self.store);
                let view = ui.add(house::quiet_with(Some("filter_list"), "View", lit));
                house::target(&ctx, "photos:view-menu", view.rect);
                egui::Popup::menu(&view).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    house::menu(ui, 280.0);
                    self.view_menu_ui(ui, &ctx, &items, folder.as_deref());
                });
                if grid {
                    let up = ui.add(house::secondary_in("Upload", theme::BLUE));
                    house::target(&ctx, "photos:upload", up.rect);
                    self.photos.up.btn = Some([up.rect.center().x, up.rect.center().y]);
                    if up.clicked() {
                        self.photos.upload_pick(&ctx, &self.store);
                    }
                }
            });
        });
    }

    /// The rows of the View menu (see [`view_items`]).
    fn view_menu_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, items: &[ViewItem], folder: Option<&str>) {
        for it in items {
            match *it {
                ViewItem::Head(h) => {
                    let r = ui.horizontal(|ui| house::caps(ui, h)).response;
                    house::target(ctx, &format!("photos:view:{}", h.to_lowercase()), r.rect);
                }
                ViewItem::Sep => house::menu_sep(ui),
                ViewItem::Set => self.scope_rows(ui, ctx),
                ViewItem::Types => self.type_rows(ui),
                ViewItem::Sort => {
                    for (newest, label) in [(true, "Newest first"), (false, "Oldest first")] {
                        let on = self.photos.newest_first == newest;
                        if house::menu_row(ui, Some(if on { "check" } else { "" }), label, "", true)
                            && !on
                            && let Some(f) = folder
                        {
                            self.photos.set_newest(ctx, &self.store, f, newest);
                        }
                    }
                }
                ViewItem::Theme => self.theme_row(ui),
                ViewItem::Upload => {
                    if house::menu_row(ui, Some("upload"), "Upload", "", true) {
                        self.photos.upload_pick(ctx, &self.store);
                    }
                }
                ViewItem::Slideshow => {
                    if house::menu_row(ui, Some("slideshow"), "Slideshow", "", true) {
                        self.photos.slideshow(ctx);
                    }
                }
                ViewItem::Scan => {
                    ui.horizontal_wrapped(|ui| self.photos.alb.ppl.scan.ui(ui, &self.store));
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The View menu: Show, Type, Sort, Theme in that order; Sort only with a picture grid; the
    /// narrow overflow menu adds Upload, Slideshow and Scan now (Scan now always).
    #[test]
    fn the_view_menu_has_show_type_sort_theme() {
        let heads = |v: Vec<ViewItem>| v.into_iter().filter_map(|i| if let ViewItem::Head(h) = i { Some(h) } else { None }).collect::<Vec<_>>();
        assert_eq!(heads(view_items(false, true, true)), ["Show", "Type", "Sort", "Theme"]);
        assert_eq!(heads(view_items(false, false, false)), ["Show", "Type", "Theme"]);
        let wide = view_items(false, true, true);
        assert!(!wide.contains(&ViewItem::Scan) && !wide.contains(&ViewItem::Upload));
        let narrow = view_items(true, true, true);
        assert!(narrow.ends_with(&[ViewItem::Upload, ViewItem::Slideshow, ViewItem::Scan]));
        assert!(view_items(true, false, false).ends_with(&[ViewItem::Sep, ViewItem::Scan]));
    }
}
