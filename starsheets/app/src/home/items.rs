//! The home screen's main area: folders and spreadsheets as cards or a list, selecting, renaming.

use super::*;

impl Home {
    pub(super) fn main_ui(&mut self, ui: &mut Ui) {
        if let Some(folder) = self.photo_folder() {
            self.photo_main_ui(ui, &folder);
            return;
        }
        // Path (each part clickable and a drop target), view switch.
        // Anywhere empty in this area: right-click for New (items drawn later sit on top of it).
        let bg = ui.interact(ui.max_rect(), egui::Id::unique("home_bg"), Sense::click());
        let searching = !self.search.trim().is_empty();
        let crumbs = if searching { vec![("Search results".to_string(), None)] } else { self.title() };
        ui.horizontal(|ui| {
            // Back, forward and up (the mouse's back button goes up too).
            let nav = |ui: &mut Ui, icon: &str, enabled: bool, tip: &str| {
                let b = match crate::icons::icon(icon) {
                    Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(22.0, 22.0))),
                    None => egui::Button::new(tip),
                };
                ui.add_enabled(enabled, b.frame(false).min_size(vec2(36.0, 36.0)).corner_radius(18))
                    .on_hover_text(tip)
                    .on_disabled_hover_text(tip)
                    .clicked()
            };
            if nav(ui, "arrow_back", !self.back.is_empty(), "Back (your mouse's back button)") {
                self.go_back();
            }
            if nav(ui, "arrow_forward", !self.forward.is_empty(), "Forward") {
                self.go_forward();
            }
            if nav(ui, "arrow_upward", self.up_view().is_some(), "Up one folder (Backspace)") {
                self.go_up();
            }
            ui.add_space(house::S);
            let last = crumbs.len().saturating_sub(1);
            for (i, (label, view)) in crumbs.into_iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("›").size(20.0).color(theme::text_dim()));
                }
                let r = ui.add(
                    egui::Button::new(RichText::new(&label).size(22.0).color(if i == last { theme::text() } else { theme::text_dim() })).frame(false),
                );
                let folder = match &view {
                    Some(View::Mine(p) | View::Everyone(p)) if !p.is_empty() => Some(p.clone()),
                    Some(View::Shared(Some(p))) => Some(p.clone()),
                    _ => None,
                };
                if self.dragging.is_some()
                    && r.contains_pointer()
                    && let Some(f) = folder
                {
                    ui.painter().rect_stroke(r.rect, 6, Stroke::new(2.0, ACCENT), egui::StrokeKind::Outside);
                    ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("home_drop"), f));
                }
                if r.clicked()
                    && let Some(v) = view
                {
                    self.navigate(v);
                }
            }
            if searching
                && ui
                    .add(egui::Button::new(RichText::new("×").size(22.0).color(theme::text_dim())).frame(false))
                    .on_hover_text("Clear search")
                    .clicked()
            {
                self.search.clear();
                self.filters = Default::default();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let toggle = |ui: &mut Ui, icon: &str, on: bool, tip: &str| {
                    let b = match crate::icons::icon(icon) {
                        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                        None => egui::Button::new(tip),
                    };
                    let r = ui
                        .add(b.fill(if on { theme::drive_selection() } else { theme::card() }).corner_radius(16).min_size(vec2(44.0, 32.0)))
                        .on_hover_text(tip);
                    house::target(ui.ctx(), &format!("drive:{icon}"), r.rect);
                    r.clicked()
                };
                let photos = self.photo_folder().is_some();
                // [Grid | ▾] and [List | ▾]: the ▾ holds that view's options (viewopts.rs).
                let (grid, more) = self.view_button(ui, "grid_view", !self.list && !photos, "Grid");
                self.grid_menu(&more);
                if grid {
                    self.list = false;
                    self.store.set_setting("starsheets_home_list", "0");
                    self.photos.set_on(&self.store, false);
                }
                ui.add_space(house::S);
                let (list, more) = self.view_button(ui, "view_list", self.list && !photos, "List");
                self.list_menu(&more);
                if list {
                    self.list = true;
                    self.store.set_setting("starsheets_home_list", "1");
                    self.photos.set_on(&self.store, false);
                }
                if self.photos_available() && toggle(ui, "photo_mode", photos, "Photos") {
                    self.photo_nav = Default::default();
                    self.photos.go(photos::albums::Tab::Photos);
                    self.photos.set_on(&self.store, true);
                    self.search.clear();
                }
                ui.add_space(house::S);
                self.sort_menu(ui);
            });
        });
        if let Some(e) = self.error.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                if ui.add(house::link("OK")).clicked() {
                    self.error = None;
                }
            });
        }
        if !self.search.trim().is_empty() {
            self.filter_chips(ui);
        }
        ui.add_space(house::M);
        if self.view == Some(View::Bin) && self.search.trim().is_empty() {
            self.bin_ui(ui);
            return;
        }
        if self.view == Some(View::Storage) {
            self.storage_header(ui);
        }
        let (folders, files) = self.contents();
        let ctx = ui.ctx().clone();
        let mut clicked_item = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let a_folder = matches!(self.view, Some(View::Mine(_) | View::Everyone(_) | View::Shared(Some(_))));
            if folders.is_empty() && files.is_empty() && a_folder && !self.loading && self.search.trim().is_empty() {
                let here = self.here();
                if empty_folder(ui, here.is_some())
                    && let Some(folder) = here
                {
                    super::templates::open(ui.ctx(), &folder);
                }
            } else if folders.is_empty() && files.is_empty() {
                let empty = if self.loading {
                    "Loading…"
                } else if !self.search.trim().is_empty() {
                    "Nothing matches."
                } else {
                    match self.view {
                        Some(View::Shared(None)) => {
                            "Nothing has been shared with you yet. When someone shares a file or folder with you, it shows here."
                        }
                        Some(View::Starred) => "Nothing starred yet. Right-click a file or folder and choose Star to keep it here.",
                        Some(View::Recent) => "Files you open show here.",
                        _ => "Nothing here yet.",
                    }
                };
                ui.add_space(house::XL);
                ui.label(RichText::new(empty).size(house::BODY).color(theme::text_dim()));
            }
            let order = self.shown(&folders, &files);
            self.card_sub = files.iter().any(|f| !self.card_sub_text(f).is_empty());
            if (self.list || self.view == Some(View::Storage)) && !order.is_empty() {
                self.list_ui(ui, &order, &mut clicked_item);
            } else if self.mixed && !matches!(self.view, Some(View::Recent | View::Storage)) {
                // Folders mixed in with files: everything as cards, in the sort's order.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(house::M, house::M);
                    for item in &order {
                        match item {
                            Item::Folder(p) => {
                                if let Some(f) = folders.iter().find(|f| f.path == *p).cloned() {
                                    self.folder_card(ui, &f, &mut clicked_item);
                                }
                            }
                            Item::File(id) => {
                                if let Some(f) = files.iter().find(|f| f.id == *id).cloned() {
                                    self.file_card(ui, &ctx, &f, &mut clicked_item);
                                }
                            }
                        }
                    }
                });
            } else {
                if !folders.is_empty() {
                    house::label(ui, "Folders", "");
                    ui.add_space(house::S);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(house::M, house::M);
                        for f in &folders {
                            self.folder_chip(ui, f, &mut clicked_item);
                        }
                    });
                    ui.add_space(house::XL);
                }
                if !files.is_empty() {
                    house::label(ui, "Files", "");
                    ui.add_space(house::S);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(house::M, house::M);
                        for f in &files {
                            self.file_card(ui, &ctx, f, &mut clicked_item);
                        }
                    });
                }
            }
        });
        // Empty space: a click clears the selection; a right-click offers New and Paste (into the
        // folder shown, or your own folder where nothing can be made, e.g. Recent).
        if bg.clicked() && !clicked_item {
            self.selected.clear();
        }
        if !clicked_item {
            bg.context_menu(|ui| {
                house::menu(ui, 260.0);
                let here = self.here();
                let Some(into) = here.clone().or_else(|| self.mine_root().map(str::to_string)) else { return };
                if house::menu_row(ui, Some("note_add"), "New spreadsheet", "", true) {
                    self.new_file(&into, false);
                }
                if house::menu_row(ui, Some("description"), "New document", "", true) {
                    self.new_file(&into, true);
                }
                if self.store.has_slides()
                    && house::menu_row(ui, Some("slideshow"), "New presentation", "", true)
                    && let Err(e) = self.store.new_presentation(&into)
                {
                    self.error = Some(e);
                }
                if house::menu_row(ui, Some("create_new_folder"), "New folder", "", true) {
                    if here.is_none() {
                        self.navigate(View::Mine(into.clone()));
                    }
                    call(&self.store, &self.inbox, ui.ctx(), Method::Post, format!("api/folders?path={}&name=New%20folder", enc(&into)), None, |b| {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        Reply::NewFolder(v["path"].as_str().unwrap_or("").to_string())
                    });
                    ui.close();
                }
                house::menu_sep(ui);
                if house::menu_row(ui, Some("content_paste"), "Paste", "Ctrl+V", self.clipboard.is_some() && here.is_some()) {
                    self.paste_into(ui.ctx(), into.clone());
                    ui.close();
                }
            });
        }
    }

    pub(super) fn select_click(&mut self, ui: &Ui, item: &Item, resp: &egui::Response, order: &[Item]) {
        if !(resp.clicked() || resp.secondary_clicked()) {
            return;
        }
        let selected = self.selected.contains(item);
        let m = ui.input(|i| i.modifiers);
        if resp.secondary_clicked() && selected {
        } else if m.command {
            if selected {
                self.selected.retain(|x| x != item);
            } else {
                self.selected.push(item.clone());
            }
            self.anchor = Some(item.clone());
        } else if m.shift
            && let (Some(a), Some(b)) = (self.anchor.as_ref().and_then(|a| order.iter().position(|x| x == a)), order.iter().position(|x| x == item))
        {
            let (lo, hi) = (a.min(b), a.max(b));
            self.selected = order[lo..=hi].to_vec();
        } else {
            self.selected = vec![item.clone()];
            self.anchor = Some(item.clone());
        }
    }

    pub(super) fn order(&self) -> Vec<Item> {
        let (folders, files) = self.contents();
        self.shown(&folders, &files)
    }

    /// Common handling of a folder or file shown anywhere: select, open, drag, menu, drop.
    pub(super) fn interact(&mut self, ui: &mut Ui, item: &Item, resp: &egui::Response, clicked_item: &mut bool) {
        if resp.clicked() || resp.secondary_clicked() || resp.double_clicked() {
            *clicked_item = true;
        }
        house::target(ui.ctx(), &format!("item:{}", self.label_of(item)), resp.rect);
        if let Item::File(id) = item {
            self.tile_rects.insert(id.clone(), resp.rect);
        }
        let order = self.order();
        self.select_click(ui, item, resp, &order);
        if resp.double_clicked() {
            self.activate(ui.ctx(), item);
        }
        if resp.drag_started() {
            let set = if self.selected.contains(item) { self.selected.clone() } else { vec![item.clone()] };
            self.selected = set.clone();
            self.dragging = Some(set);
        }
        if let Item::Folder(f) = item
            && self.dragging.as_ref().is_some_and(|d| !d.contains(item))
            && resp.contains_pointer()
        {
            ui.painter().rect_stroke(resp.rect, 8, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
            ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("home_drop"), f.clone()));
        }
        resp.context_menu(|ui| {
            *clicked_item = true;
            self.item_menu(ui, item);
        });
    }

    pub(super) fn folder_chip(&mut self, ui: &mut Ui, f: &FolderRow, clicked_item: &mut bool) {
        let item = Item::Folder(f.path.clone());
        let (cw, _, _) = self.vo.card_with(self.vo.dates || self.card_sub);
        let (r, resp) = ui.allocate_exact_size(vec2(cw, CHIP_H), Sense::click_and_drag());
        let sel = self.selected.contains(&item);
        let p = ui.painter().clone();
        p.rect_filled(
            r,
            12,
            if sel {
                theme::drive_selection()
            } else if resp.hovered() {
                theme::hover()
            } else {
                theme::drive_surface()
            },
        );
        folder_icon(ui, f, Rect::from_center_size(r.left_center() + vec2(24.0, 0.0), vec2(22.0, 22.0)));
        if f.shared
            && f.colour.is_some()
            && let Some(img) = crate::icons::icon("group")
        {
            img.paint_at(ui, Rect::from_center_size(r.right_center() - vec2(if f.starred { 40.0 } else { 18.0 }, 0.0), vec2(16.0, 16.0)));
        }
        if !self.renaming_ui(ui, &item, Rect::from_min_max(r.min + vec2(44.0, 8.0), r.max - vec2(8.0, 8.0))) {
            let galley = p.layout(self.folder_label(&f.path), FontId::proportional(14.0), theme::text(), cw - 76.0);
            p.galley(egui::pos2(r.left() + 44.0, r.center().y - galley.size().y.min(20.0) / 2.0), galley, theme::text());
        }
        if f.starred
            && let Some(img) = crate::icons::icon("star_fill")
        {
            img.paint_at(ui, Rect::from_center_size(r.right_center() - vec2(18.0, 0.0), vec2(16.0, 16.0)));
        }
        let hover = if self.accounts && f.owner != self.space {
            format!("{} · {}", name_of(&f.path), self.person(&f.owner))
        } else {
            name_of(&f.path).to_string()
        };
        let resp = resp.on_hover_text(hover);
        self.interact(ui, &item, &resp, clicked_item);
    }

    /// A folder the size of a file card (when folders are mixed in with files).
    pub(super) fn folder_card(&mut self, ui: &mut Ui, f: &FolderRow, clicked_item: &mut bool) {
        let item = Item::Folder(f.path.clone());
        let (cw, ch, ph) = self.vo.card_with(self.vo.dates || self.card_sub);
        let (r, resp) = ui.allocate_exact_size(vec2(cw, ch), Sense::click_and_drag());
        let sel = self.selected.contains(&item);
        let p = ui.painter().clone();
        p.rect_filled(
            r,
            12,
            if sel {
                theme::drive_selection()
            } else if resp.hovered() {
                theme::hover()
            } else {
                theme::drive_surface()
            },
        );
        let pic = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(cw - 16.0, ph - 8.0));
        p.rect_filled(pic, 6, Color32::WHITE);
        folder_icon(ui, f, Rect::from_center_size(pic.center(), vec2(64.0, 64.0)));
        let line = Rect::from_min_max(egui::pos2(r.left() + 8.0, r.top() + ph), r.max - vec2(8.0, 4.0));
        folder_icon(ui, f, Rect::from_center_size(egui::pos2(line.left() + 10.0, line.top() + 16.0), vec2(18.0, 18.0)));
        if !self.renaming_ui(ui, &item, Rect::from_min_size(egui::pos2(line.left() + 26.0, line.top() + 4.0), vec2(line.width() - 30.0, 26.0))) {
            let name = one_line(&p, self.folder_label(&f.path), 14.0, theme::text(), line.width() - 52.0);
            p.galley(egui::pos2(line.left() + 28.0, line.top() + 6.0), name, theme::text());
        }
        let (newest, _) = self.folder_stats(&f.path);
        if self.vo.dates {
            let sub = if newest > 0 { format!("Folder · {}", ago(newest)) } else { "Folder".into() };
            p.text(egui::pos2(line.left() + 28.0, line.top() + 36.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), theme::text_dim());
        }
        let resp = resp.on_hover_text(self.folder_label(&f.path));
        self.interact(ui, &item, &resp, clicked_item);
    }

    pub(super) fn file_card(&mut self, ui: &mut Ui, ctx: &egui::Context, f: &FileRow, clicked_item: &mut bool) {
        let item = Item::File(f.id.clone());
        let kind = kind_icon(&f.name);
        let (cw, ch, ph) = self.vo.card_with(self.vo.dates || self.card_sub);
        let (r, resp) = ui.allocate_exact_size(vec2(cw, ch), Sense::click_and_drag());
        if ui.is_rect_visible(r) {
            self.want_thumb(ctx, f);
        }
        let sel = self.selected.contains(&item);
        let p = ui.painter().clone();
        p.rect_filled(
            r,
            12,
            if sel {
                theme::drive_selection()
            } else if resp.hovered() {
                theme::hover()
            } else {
                theme::drive_surface()
            },
        );
        // The picture, on white, in the top part.
        let pic = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(cw - 16.0, ph - 8.0));
        p.rect_filled(pic, 6, Color32::WHITE);
        match self.thumbs.get(&f.id).map(|(_, t)| t) {
            Some(Thumb::Ready(tex)) if self.vo.previews && file_kind(&f.name) == "video" => {
                let size = tex.size_vec2();
                let scale = (pic.width() / size.x).max(pic.height() / size.y);
                let visible = vec2(pic.width() / (size.x * scale), pic.height() / (size.y * scale));
                let uv = Rect::from_center_size(egui::pos2(0.5, 0.5), visible);
                p.image(tex.id(), pic, uv, Color32::WHITE);
            }
            Some(Thumb::Ready(tex)) if self.vo.previews => {
                // The picture's full width (columns A to L, as Drive shows), cut off at the bottom.
                let tw = tex.size_vec2();
                let scale = pic.width() / tw.x;
                let uv = Rect::from_min_max(
                    egui::pos2(0.0, 0.0),
                    egui::pos2((pic.width() / (tw.x * scale)).min(1.0), (pic.height() / (tw.y * scale)).min(1.0)),
                );
                p.image(tex.id(), pic, uv, Color32::WHITE);
            }
            _ => {
                if let Some(img) = crate::icons::icon(kind) {
                    img.paint_at(ui, Rect::from_center_size(pic.center(), vec2(48.0, 48.0)));
                }
            }
        }
        p.rect_stroke(pic, 6, Stroke::new(1.0, theme::divider()), egui::StrokeKind::Inside);
        let line = Rect::from_min_max(egui::pos2(r.left() + 8.0, r.top() + ph), r.max - vec2(8.0, 4.0));
        if let Some(img) = crate::icons::icon(kind) {
            img.paint_at(ui, Rect::from_center_size(egui::pos2(line.left() + 10.0, line.top() + 16.0), vec2(18.0, 18.0)));
        }
        if !self.renaming_ui(ui, &item, Rect::from_min_size(egui::pos2(line.left() + 26.0, line.top() + 4.0), vec2(line.width() - 30.0, 26.0))) {
            let name = one_line(&p, shown_name(&f.name).to_string(), 14.0, theme::text(), line.width() - 52.0);
            p.galley(egui::pos2(line.left() + 28.0, line.top() + 6.0), name, theme::text());
        }
        if f.starred
            && let Some(img) = crate::icons::icon("star_fill")
        {
            img.paint_at(ui, Rect::from_center_size(egui::pos2(line.right() - 10.0, line.top() + 16.0), vec2(16.0, 16.0)));
        }
        let sub = self.card_sub_text(f);
        let has_sub = !sub.is_empty();
        if has_sub {
            // Cut to the card's width: a long snippet must not run past the card.
            let g = one_line(&p, sub, 12.0, theme::text_dim(), line.width() - 52.0);
            p.galley(egui::pos2(line.left() + 28.0, line.top() + 36.0 - g.size().y / 2.0), g, theme::text_dim());
        }
        if f.shared
            && let Some(img) = crate::icons::icon("group")
        {
            // Without a second line the shared mark sits on the name line, left of the star.
            let (x, y) = if has_sub { (line.right() - 10.0, line.top() + 36.0) } else { (line.right() - 30.0, line.top() + 16.0) };
            img.paint_at(ui, Rect::from_center_size(egui::pos2(x, y), vec2(14.0, 14.0)));
        }
        let resp = resp.on_hover_text(&f.name);
        self.interact(ui, &item, &resp, clicked_item);
    }

    /// A file card's second line: the date line (when shown), a search snippet, or the owner's name; empty when none applies.
    pub(super) fn card_sub_text(&self, f: &FileRow) -> String {
        match (&self.view, f.opened) {
            (Some(View::Recent), Some(t)) if self.vo.dates => format!("You opened {}", ago(t)),
            _ if self.found_snippet(&f.id).is_some() => self.found_snippet(&f.id).unwrap_or_default().to_string(),
            _ if self.accounts && f.owner != self.space && !self.vo.dates => self.person(&f.owner),
            _ if self.accounts && f.owner != self.space => format!("{} · {}", self.person(&f.owner), ago(f.modified)),
            _ if !self.vo.dates => String::new(),
            _ => format!("Modified {}", ago(f.modified)),
        }
    }

    /// The words around the match inside this file, while a search that found text in it is shown.
    pub(super) fn found_snippet(&self, id: &str) -> Option<&str> {
        let q = self.search.trim();
        if q.is_empty() || self.found_for != q {
            return None;
        }
        self.found.get(id).map(String::as_str).filter(|s| !s.is_empty())
    }

    pub(super) fn list_ui(&mut self, ui: &mut Ui, order: &[Item], clicked_item: &mut bool) {
        let w = ui.available_width() - 8.0;
        let (h, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
        let storage = self.view == Some(View::Storage);
        // The columns chosen in List ▾ (viewopts.rs); drag the left edge of one to resize it.
        let cols = self.vo.layout(&self.list_have(), w);
        let name_w = cols[0].2;
        let x_of = |key: &str| cols.iter().find(|c| c.0 == key).map(|c| (c.1, c.2));
        for &(key, x, cw) in &cols {
            ui.painter().text(
                h.left_center() + vec2(x + if key == "name" { 44.0 } else { 0.0 }, 0.0),
                Align2::LEFT_CENTER,
                super::viewopts::col_label(key, storage),
                FontId::proportional(13.0),
                theme::text_dim(),
            );
            if key == "name" {
                continue;
            }
            let edge = Rect::from_center_size(egui::pos2(h.left() + x - 6.0, h.center().y), vec2(10.0, h.height()));
            let r = ui.interact(edge, egui::Id::unique(("home_col_edge", key)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            house::target(ui.ctx(), &format!("List column edge {}", super::viewopts::col_label(key, storage)), edge);
            if r.hovered() || r.dragged() {
                ui.painter().vline(edge.center().x, h.y_range(), Stroke::new(2.0, theme::divider()));
            }
            if r.dragged() {
                // Left makes this column wider (and Name narrower), never past either's minimum.
                let most = cw + (name_w - super::viewopts::NAME_MIN).max(0.0);
                self.vo.set_width(key, (cw - r.drag_delta().x).clamp(super::viewopts::COL_MIN, most.max(super::viewopts::COL_MIN)));
            }
            if r.drag_stopped() {
                self.vo.save(&self.store);
            }
        }
        ui.painter().hline(h.x_range(), h.bottom(), Stroke::new(1.0, theme::divider()));
        let rows: Vec<ListRow> = order
            .iter()
            .filter_map(|item| match item {
                Item::Folder(p) => self.folder(p).map(|f| {
                    let (newest, size) = self.folder_stats(&f.path);
                    ListRow {
                        item: item.clone(),
                        name: self.folder_label(&f.path),
                        icon: if f.shared { "folder_shared" } else { "folder" },
                        owner: f.owner.clone(),
                        modified: newest,
                        size: Some(size),
                        starred: f.starred,
                    }
                }),
                Item::File(id) => self.file(id).map(|f| ListRow {
                    item: item.clone(),
                    name: f.name.clone(),
                    icon: kind_icon(&f.name),
                    owner: f.owner.clone(),
                    modified: f.modified,
                    size: Some(f.size),
                    starred: f.starred,
                }),
            })
            .collect();
        for ListRow { item, name, icon, owner, modified, size, starred } in rows {
            let snippet = match &item {
                Item::File(id) => self.found_snippet(id).map(str::to_string),
                Item::Folder(_) => None,
            };
            let (r, resp) = ui.allocate_exact_size(vec2(w, ROW), Sense::click_and_drag());
            let p = ui.painter().clone();
            if self.selected.contains(&item) {
                p.rect_filled(r, 6, house::SELECTED);
            } else if resp.hovered() {
                p.rect_filled(r, 6, theme::menu_hover());
            }
            let icon_rect = Rect::from_center_size(r.left_center() + vec2(20.0, 0.0), vec2(20.0, 20.0));
            match &item {
                Item::Folder(p) if let Some(f) = self.folder(p).cloned() => folder_icon(ui, &f, icon_rect),
                _ => {
                    if let Some(img) = crate::icons::icon(icon) {
                        img.paint_at(ui, icon_rect);
                    }
                }
            }
            let mut resp = resp;
            if !self.renaming_ui(ui, &item, Rect::from_min_size(r.min + vec2(40.0, 5.0), vec2(name_w - 48.0, ROW - 10.0))) {
                // Cut short with "…" before the next column (the full name on hover), ★ after it.
                let room = name_w - 44.0 - house::L - if starred { 24.0 } else { 0.0 };
                let shown = if matches!(item, Item::File(_)) { shown_name(&name) } else { &name };
                let galley = one_line(&p, shown.to_string(), 14.0, theme::text(), room);
                let at = r.left_center() + vec2(44.0, -galley.size().y / 2.0);
                let end = at.x + galley.size().x;
                if galley.elided {
                    resp = resp.on_hover_text(&name);
                }
                p.galley(at, galley, theme::text());
                let star_room = if starred { 24.0 } else { 0.0 };
                if starred {
                    p.text(egui::pos2(end + 8.0, r.center().y), Align2::LEFT_CENTER, "★", FontId::proportional(14.0), theme::text());
                }
                // "@ name": that person mentioned you in a comment (gone once you open the file).
                if let Item::File(id) = &item
                    && let Some(f) = self.file(id).filter(|f| !f.mentioned_by.is_empty())
                {
                    let text = format!("@ {} mentioned you", self.person(&f.mentioned_by));
                    p.text(egui::pos2(end + 8.0 + star_room, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), house::BLUE);
                } else if let Some(s) = snippet {
                    // A match inside the file: the words around it, dim, after the name.
                    let x = end + if starred { 30.0 } else { 14.0 };
                    let left = r.left() + room + 44.0 - x;
                    if left > 40.0 {
                        let g = one_line(&p, s, 12.0, theme::text_dim(), left);
                        p.galley(egui::pos2(x, r.center().y - g.size().y / 2.0), g, theme::text_dim());
                    }
                }
            }
            let owner_col = x_of("owner");
            if storage && let Some((ox, ow)) = owner_col {
                let place = match &item {
                    Item::File(id) => self.file(id).map(|f| f.folder.clone()).unwrap_or_default(),
                    Item::Folder(_) => String::new(),
                };
                let place =
                    if self.accounts { place.strip_prefix(self.space.as_str()).unwrap_or(&place).trim_start_matches('/').to_string() } else { place };
                let place = if place.is_empty() { "My files".to_string() } else { place };
                let place = one_line(&p, place, 13.0, theme::text_dim(), ow - house::M);
                p.galley(r.left_center() + vec2(ox, -7.0), place, theme::text_dim());
            } else if let Some((ox, ow)) = owner_col {
                let who = one_line(&p, self.person(&owner), 13.0, theme::text_dim(), ow - house::M);
                p.galley(r.left_center() + vec2(ox, -who.size().y / 2.0), who, theme::text_dim());
            }
            // Each column cut short before the next one.
            let cell = |key: &str, text: String| {
                if let Some((x, cw)) = x_of(key) {
                    let g = one_line(&p, text, 13.0, theme::text_dim(), cw - house::M);
                    p.galley(r.left_center() + vec2(x, -g.size().y / 2.0), g, theme::text_dim());
                }
            };
            if modified > 0 {
                cell("modified", ago(modified));
            }
            cell("type", super::viewopts::type_label(&item, &name).to_string());
            if let Some(s) = size {
                cell("size", size_text(s));
            }
            self.interact(ui, &item, &resp, clicked_item);
        }
    }

    /// The rename box over `rect` if `item` is being renamed; true when it is.
    pub(super) fn renaming_ui(&mut self, ui: &mut Ui, item: &Item, rect: Rect) -> bool {
        let Some((ritem, text, focus)) = self.renaming.as_mut().filter(|(r, ..)| r == item) else { return false };
        // A child Ui, not scope_builder: drawn over the card or row at an offset, it must not move
        // the parent's layout cursor (that lifted a new folder a few pixels above the others on
        // its row, to-do 3 row 15).
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        house::form_style(&mut child);
        let te = child.add_sized(rect.size(), house::field(text, house::BODY));
        if *focus {
            te.request_focus();
            // The whole name is selected, so typing replaces it (as in Drive).
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), te.id) {
                let n = text.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                st.store(ui.ctx(), te.id);
            } else {
                // The state exists from the box's first frame on: select on the next one.
                ui.ctx().request_repaint();
                return true;
            }
            *focus = false;
        }
        if te.lost_focus() {
            let (ritem, text) = (ritem.clone(), text.trim().to_string());
            let commit = !ui.input(|i| i.key_pressed(Key::Escape));
            // The Enter that names it must not also open the folder (the home screen's keys run
            // after this and would see it with nothing focused).
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
            self.renaming = None;
            let naming = matches!(&ritem, Item::Folder(p) if self.just_made.as_ref() == Some(p));
            self.just_made = None;
            if commit && !text.is_empty() {
                // Undoable (undo.rs); naming a folder just made joins its New folder's undo.
                let group = if naming { self.undo.start_naming() } else { self.undo.start("Renamed".into(), 1, false) };
                let (url, then): (String, super::undo::Then) = match &ritem {
                    Item::File(id) => {
                        let (id, was) = (id.clone(), self.file(id).map(|f| join(&f.folder, &f.name)).unwrap_or_default());
                        (
                            format!("api/files/{id}/rename?name={}", enc(&text)),
                            Box::new(move |v: &Json| {
                                let now = join(parent_of(&was), v["name"].as_str()?);
                                let id = v["id"].as_str().map_or(id, str::to_string);
                                (!was.is_empty() && now != was).then_some(super::undo::Step::File { id, was, now })
                            }),
                        )
                    }
                    Item::Folder(p) => {
                        let was = p.clone();
                        (
                            format!("api/folders/rename?path={}&name={}", enc(p), enc(&text)),
                            Box::new(move |v: &Json| {
                                let now = v["path"].as_str()?.to_string();
                                let id = v["id"].as_str().unwrap_or_default().to_string();
                                (now != was).then_some(super::undo::Step::Folder { id, was, now })
                            }),
                        )
                    }
                };
                call_any(&self.store, &self.inbox, ui.ctx(), Method::Post, url, None, move |r| {
                    Reply::Did(group, r.map(|b| then(&serde_json::from_slice(&b).unwrap_or_default())).map_err(|e| e.1))
                });
            }
        }
        true
    }

    pub(super) fn label_of(&self, item: &Item) -> String {
        match item {
            Item::Folder(f) => name_of(f).to_string(),
            Item::File(id) => self.file(id).map_or_else(String::new, |f| f.name.clone()),
        }
    }

    pub(super) fn role_of(&self, item: &Item) -> &str {
        match item {
            Item::Folder(f) => self.folder(f).map_or("owner", |f| f.role.as_str()),
            Item::File(id) => self.file(id).map_or("owner", |f| f.role.as_str()),
        }
    }

    pub(super) fn activate(&mut self, ctx: &egui::Context, item: &Item) {
        match item {
            Item::Folder(f) => {
                let f = f.clone();
                self.go_folder(&f);
            }
            // Files always open in a new tab, as from Google Drive, so the file browser stays
            // where it was (to-do 2 row 5; the separate "Open in new tab" entry went with it).
            // Other files: Drive's preview over the list (pictures, PDFs, sound, films, text; else
            // "No preview available" with Download).
            Item::File(id) => {
                let name = self.file(id).map(|f| f.name.clone()).unwrap_or_default();
                // This computer's own files open where they belong: Sheets, Docs, else the
                // system's default app (localstore.rs).
                let editor = opens_as(&name) == Opens::Editor || self.store.opens_any_file();
                match if editor { Opens::Editor } else { opens_as(&name) } {
                    Opens::Editor => {
                        if let Err(e) = self.store.open(id, &name) {
                            self.error = Some(e);
                        }
                    }
                    // Anything else: Drive's preview ("No preview available" for unknown types).
                    Opens::Preview | Opens::Download => self.preview(ctx, id),
                }
            }
        }
    }

    pub(super) fn start_rename(&mut self, item: &Item) {
        let name = self.label_of(item);
        let shown = match item {
            Item::File(_) => name.rsplit_once('.').map_or(name.clone(), |(s, _)| s.to_string()),
            Item::Folder(_) => name,
        };
        self.renaming = Some((item.clone(), shown, true));
    }
}

/// What a file is, from its name: "sheet", "doc", or for other files (the server's own list is
/// crates/server/src/preview.rs) "image", "audio", "video", "pdf", "text" or "other" (HTML, SVG
/// and anything unknown: download only, never shown).
pub(super) fn file_kind(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if crate::store::is_doc(&lower) {
        return "doc";
    }
    if [".xlsx", ".xlsm", ".xls", ".csv", ".ods"].iter().any(|x| lower.ends_with(x)) {
        return "sheet";
    }
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "avif" | "bmp" | "ico" | "tif" | "tiff" => "image",
        "mp3" | "wav" | "ogg" | "oga" | "opus" | "flac" | "m4a" | "aac" => "audio",
        "mp4" | "m4v" | "webm" | "ogv" | "mov" => "video",
        "pdf" => "pdf",
        "txt" | "text" | "md" | "markdown" | "log" | "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "env" | "tsv" | "rs" | "py"
        | "js" | "mjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cc" | "cpp" | "hpp" | "java" | "kt" | "go" | "rb" | "php" | "sh" | "bash" | "zsh"
        | "fish" | "ps1" | "bat" | "css" | "scss" | "sql" | "lua" | "pl" | "r" | "swift" | "cs" | "diff" | "patch" | "tex" | "srt" | "vtt"
        | "nfo" => "text",
        _ => "other",
    }
}

/// Pictures supported by the server's unindexed 1024-pixel viewer endpoint.
pub(super) fn photo_preview(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"))
}

/// The icon of a file's type.
pub(super) fn kind_icon(name: &str) -> &'static str {
    match file_kind(name) {
        "sheet" | "doc" => crate::store::app_icon(name),
        "image" => "image",
        "audio" => "music_note",
        "video" => "movie",
        "pdf" => "picture_as_pdf",
        "text" => "description",
        _ => "draft",
    }
}

/// What opening a file does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Opens {
    /// Starbase Sheets or Docs.
    Editor,
    /// Drive's preview over the file list (filepreview.rs).
    Preview,
    Download,
}

pub(super) fn opens_as(name: &str) -> Opens {
    match file_kind(name) {
        "sheet" | "doc" => Opens::Editor,
        "other" => Opens::Download,
        _ => Opens::Preview,
    }
}

/// One row of the list view: a folder or a file, as it is shown.
struct ListRow {
    item: Item,
    name: String,
    icon: &'static str,
    owner: String,
    modified: i64,
    size: Option<u64>,
    starred: bool,
}

/// A file's name as Drive shows it: without `.xlsx`, `.docx` or `.pptx` (the icon says which it is; the
/// full name shows on hover). Other extensions (.csv, .odt) stay.
fn shown_name(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    if (lower.ends_with(".xlsx") || lower.ends_with(".docx") || lower.ends_with(".pptx")) && name.len() > 5 { &name[..name.len() - 5] } else { name }
}

/// `text` on one line, at most `width` wide: cut short with "…" when longer (as Drive does).
fn one_line(p: &egui::Painter, text: String, size: f32, colour: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(size), colour);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    p.layout_job(job)
}

/// An empty folder, as in Google Drive: a round picture, "Drop files here" and how else to add
/// something; for a folder you can only view, just that it is empty.
/// The empty folder's picture and words; `true` when "Start from a template" was clicked.
fn empty_folder(ui: &mut Ui, can_add: bool) -> bool {
    ui.vertical_centered(|ui| {
        ui.add_space(64.0);
        let (r, _) = ui.allocate_exact_size(vec2(120.0, 120.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 60.0, theme::drive_surface());
        if let Some(img) = crate::icons::icon(if can_add { "upload" } else { "folder_open" }) {
            img.fit_to_exact_size(vec2(56.0, 56.0)).paint_at(ui, Rect::from_center_size(r.center(), vec2(56.0, 56.0)));
        }
        ui.add_space(house::L);
        let (head, sub) = if can_add {
            ("Drop files here", "or use the New button to make a spreadsheet, a document or a folder")
        } else {
            ("This folder is empty", "Files and folders added here will show up here.")
        };
        ui.label(RichText::new(head).size(house::TITLE).color(theme::text()));
        ui.add_space(house::XS);
        ui.label(RichText::new(sub).size(house::BODY).color(theme::text_dim()));
        can_add && {
            ui.add_space(house::S);
            let r = ui.add(house::link("Start from a template"));
            house::target(ui.ctx(), "Start from a template", r.rect);
            r.clicked()
        }
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::{Opens, kind_icon, opens_as, shown_name};

    #[test]
    fn every_file_type_has_an_icon_and_a_way_to_open() {
        assert_eq!((kind_icon("a.xlsx"), opens_as("a.xlsx")), ("starsheets", Opens::Editor));
        assert_eq!((kind_icon("a.DOCX"), opens_as("a.DOCX")), ("stardocs", Opens::Editor));
        assert_eq!((kind_icon("Talk.pptx"), opens_as("Talk.pptx")), ("starslides", Opens::Editor));
        assert_eq!((kind_icon("a.JPG"), opens_as("a.JPG")), ("image", Opens::Preview));
        assert_eq!((kind_icon("a.pdf"), opens_as("a.pdf")), ("picture_as_pdf", Opens::Preview));
        assert_eq!((kind_icon("a.mp4"), opens_as("a.mp4")), ("movie", Opens::Preview));
        assert_eq!((kind_icon("a.mp3"), opens_as("a.mp3")), ("music_note", Opens::Preview));
        assert_eq!((kind_icon("a.md"), opens_as("a.md")), ("description", Opens::Preview));
        // Never shown in the browser: download only.
        for n in ["a.html", "a.svg", "a.exe", "README"] {
            assert_eq!((kind_icon(n), opens_as(n)), ("draft", Opens::Download), "{n}");
        }
        for icon in ["starsheets", "stardocs", "starslides", "starphotos", "image", "picture_as_pdf", "movie", "music_note", "description", "draft"] {
            assert!(crate::icons::has(icon), "{icon}");
        }
    }

    #[test]
    fn drive_hides_only_its_own_file_extensions() {
        assert_eq!(shown_name("Budget.xlsx"), "Budget");
        assert_eq!(shown_name("Letter.DOCX"), "Letter");
        assert_eq!(shown_name("data.csv"), "data.csv");
        assert_eq!(shown_name("notes.odt"), "notes.odt");
        assert_eq!(shown_name(".xlsx"), ".xlsx");
    }
}
