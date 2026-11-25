//! The home screen's left side: My files, Shared with me, Recent, Starred, Bin and the folder tree.

use super::*;

impl Home {
    pub(super) fn left_ui(&mut self, ui: &mut Ui) {
        // New: a spreadsheet, a folder, or files from this computer.
        let here = self.here();
        // The house's raised button: white, 16 px corners, a soft shadow, the + icon.
        let label = RichText::new("New").size(15.0).color(theme::TEXT);
        let button = match crate::icons::icon("add") {
            Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(24.0, 24.0)), label),
            None => egui::Button::new(label),
        };
        let new = egui::Frame::NONE
            .corner_radius(16)
            .shadow(egui::epaint::Shadow { offset: [0, 1], blur: 6, spread: 0, color: Color32::from_black_alpha(50) })
            .show(ui, |ui| {
                ui.spacing_mut().button_padding = vec2(house::L, house::M);
                ui.add(button.fill(Color32::WHITE).corner_radius(16).min_size(vec2(112.0, 56.0)).stroke(Stroke::NONE))
            })
            .inner;
        egui::Popup::menu(&new).show(|ui| {
            house::menu(ui, 260.0);
            let folder = here.clone().unwrap_or_else(|| self.space.clone());
            if house::menu_row(ui, Some("note_add"), "New spreadsheet", "", true) {
                self.new_file(&folder, false);
            }
            if house::menu_row(ui, Some("description"), "New document", "", true) {
                self.new_file(&folder, true);
            }
            if house::menu_row(ui, Some("create_new_folder"), "New folder", "", true) {
                call(&self.store, &self.inbox, ui.ctx(), Method::Post, format!("api/folders?path={}&name=New%20folder", enc(&folder)), None, |b| {
                    let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                    Reply::NewFolder(v["path"].as_str().unwrap_or("").to_string())
                });
                if here.is_none() {
                    self.navigate(View::Mine(self.space.clone()));
                }
            }
            house::menu_sep(ui);
            if house::menu_row(ui, Some("upload"), "Upload files…", "", true) {
                let (store, inbox, ctx) = (self.store.clone(), self.inbox.clone(), ui.ctx().clone());
                self.store.pick_files(Box::new(move |name, bytes| {
                    let url = format!("api/files?name={}&folder={}&as_is=1", enc(&name), enc(&folder));
                    call(&store, &inbox, &ctx, Method::Post, url, Some(bytes), refresh_after);
                }));
            }
        });
        ui.add_space(house::M);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let space = self.space.clone();
            let mine_label = if self.accounts { "My files" } else { "All files" };
            self.node(ui, mine_label, "home_storage", Some(View::Mine(space.clone())), Some(&space), "mine");
            if self.accounts {
                self.node(ui, "Shared with me", "folder_shared", Some(View::Shared(None)), None, "shared");
                if self.expanded.contains("shared:") {
                    for top in self.shared_folders.clone() {
                        self.tree(ui, &top, 1, "shared", &|p| View::Shared(Some(p)));
                    }
                }
            }
            self.node(ui, "Recent", "schedule", Some(View::Recent), None, "recent");
            self.node(ui, "Starred", "star", Some(View::Starred), None, "starred");
            self.node(ui, "Bin", "delete", Some(View::Bin), None, "bin");
            if self.accounts && self.admin {
                ui.add_space(house::S);
                ui.separator();
                self.node(ui, "Everyone's files", "groups", Some(View::Everyone(String::new())), Some(""), "every");
            }
        });
    }

    /// A row of the tree. `folder`: the folder it stands for (it can be opened up and dropped on).
    pub(super) fn node(&mut self, ui: &mut Ui, label: &str, icon: &str, view: Option<View>, folder: Option<&str>, kind: &str) {
        let key = format!("{kind}:{}", folder.unwrap_or(""));
        let has_children = match (kind, folder) {
            ("shared", _) => !self.shared_folders.is_empty(),
            ("every", _) => self.folders.iter().any(|f| !f.path.contains('/')),
            (_, Some(f)) => self.folders.iter().any(|x| parent_of(&x.path) == f && x.path != f),
            _ => false,
        };
        let current = view.is_some() && self.view == view;
        if self.row(ui, label, icon, None, 0, current, has_children, &key, folder.map(str::to_string)) {
            if let Some(v) = view {
                self.navigate(v);
            }
            if self.view == Some(View::Bin) {
                self.loading = true;
                self.load_bin(ui.ctx());
            }
        }
        if self.expanded.contains(&key) {
            match (kind, folder) {
                ("every", _) => {
                    let mut tops: Vec<String> =
                        self.folders.iter().filter(|f| !f.path.contains('/') && !f.path.is_empty()).map(|f| f.path.clone()).collect();
                    tops.sort();
                    for t in tops {
                        self.tree(ui, &t, 1, "every", &View::Everyone);
                    }
                }
                (_, Some(f)) if kind == "mine" => {
                    for c in self.children(f) {
                        self.tree(ui, &c, 1, "mine", &View::Mine);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn children(&self, path: &str) -> Vec<String> {
        let mut c: Vec<String> =
            self.folders.iter().filter(|f| parent_of(&f.path) == path && f.path != path && !f.path.is_empty()).map(|f| f.path.clone()).collect();
        c.sort_by_key(|p| name_of(p).to_lowercase());
        c
    }

    pub(super) fn tree(&mut self, ui: &mut Ui, path: &str, depth: usize, kind: &str, make: &dyn Fn(String) -> View) {
        let key = format!("{kind}:{path}");
        let view = make(path.to_string());
        let kids = self.children(path);
        let label = if kind == "every" && depth == 1 {
            let p = self.person(path);
            if p == "me" { "My files".to_string() } else { p }
        } else {
            name_of(path).to_string()
        };
        let current = self.view.as_ref() == Some(&view);
        let colour = self.folder(path).and_then(|f| f.colour);
        let icon = if colour.is_some() { "folder_fill" } else { "folder" };
        if self.row(ui, &label, icon, colour, depth, current, !kids.is_empty(), &key, Some(path.to_string())) {
            self.navigate(view);
        }
        if self.expanded.contains(&key) {
            for k in kids {
                self.tree(ui, &k, depth + 1, kind, make);
            }
        }
    }

    /// Draws one tree row; returns true when it was clicked (not on its arrow).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row(
        &mut self,
        ui: &mut Ui,
        label: &str,
        icon: &str,
        tint: Option<Color32>,
        depth: usize,
        current: bool,
        has_children: bool,
        key: &str,
        folder: Option<String>,
    ) -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let p = ui.painter().clone();
        let dropping = self.dragging.is_some() && folder.is_some() && resp.contains_pointer();
        if current {
            p.rect_filled(r, 17, theme::SHARE);
        } else if dropping || resp.hovered() {
            p.rect_filled(r, 17, theme::HOVER);
        }
        if dropping {
            p.rect_stroke(r, 17, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
            ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("home_drop"), folder.clone().unwrap_or_default()));
        }
        let x = r.left() + 4.0 + depth as f32 * 16.0;
        let arrow = Rect::from_center_size(egui::pos2(x + 10.0, r.center().y), vec2(20.0, 20.0));
        let mut toggled = false;
        if has_children {
            let open = self.expanded.contains(key);
            if let Some(img) = crate::icons::icon(if open { "expand_more" } else { "chevron_right" }) {
                img.paint_at(ui, arrow);
            }
            if ui.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|pos| arrow.expand(4.0).contains(pos))) {
                toggled = true;
                if open {
                    self.expanded.remove(key);
                } else {
                    self.expanded.insert(key.to_string());
                }
            }
        }
        if let Some(img) = crate::icons::icon(icon) {
            let img = match tint {
                Some(c) => img.tint(c),
                None => img,
            };
            img.paint_at(ui, Rect::from_center_size(egui::pos2(x + 34.0, r.center().y), vec2(20.0, 20.0)));
        }
        let font = FontId::proportional(14.0);
        let galley = p.layout(label.to_string(), font, theme::TEXT, r.right() - x - 56.0);
        p.galley(egui::pos2(x + 52.0, r.center().y - galley.size().y / 2.0), galley, theme::TEXT);
        resp.clicked() && !toggled
    }
}
