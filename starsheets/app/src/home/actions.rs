//! What can be done to files and folders on the home screen: the right-click menus, folder colours,
//! deleting, copying and moving (with "Move to" and the warning before giving something away), Details.

use super::*;

impl Home {
    pub(super) fn item_menu(&mut self, ui: &mut Ui, item: &Item) {
        house::menu(ui, 280.0);
        let targets = if self.selected.contains(item) { self.selected.clone() } else { vec![item.clone()] };
        let single = targets.len() == 1;
        let role = self.role_of(item).to_string();
        let can_edit = may_edit(&role);
        let is_space = matches!(item, Item::Folder(f) if !f.contains('/') && self.accounts);
        let row = |ui: &mut Ui, icon: &str, label: &str, keys: &str, enabled: bool| house::menu_row(ui, Some(icon), label, keys, enabled);
        if row(ui, "", "Open", "Enter", single) {
            self.activate(ui.ctx(), item);
            ui.close();
        }
        house::menu_sep(ui);
        if self.accounts && row(ui, "person_add", "Share…", "", single && !is_space) {
            let (folder, key) = match item {
                Item::Folder(p) => (true, p.clone()),
                Item::File(id) => (false, id.clone()),
            };
            #[cfg(target_arch = "wasm32")]
            crate::share::share_item(ui.ctx(), folder, &key, &self.label_of(item));
            #[cfg(not(target_arch = "wasm32"))]
            let _ = (folder, key);
            ui.close();
        }
        let starred = match item {
            Item::Folder(f) => self.folder(f).is_some_and(|f| f.starred),
            Item::File(id) => self.file(id).is_some_and(|f| f.starred),
        };
        if self.accounts && row(ui, if starred { "star_fill" } else { "star" }, if starred { "Remove star" } else { "Star" }, "", true) {
            for t in &targets {
                let url = match t {
                    Item::File(id) => format!("api/files/{id}/star"),
                    Item::Folder(p) => format!("api/folders/star?path={}", enc(p)),
                };
                call(&self.store, &self.inbox, ui.ctx(), Method::Post, url, Some(format!("{{\"on\":{}}}", !starred).into_bytes()), refresh_after);
            }
            ui.close();
        }
        if let Item::Folder(path) = item
            && self.accounts
        {
            self.colour_menu(ui, path, &targets);
        }
        if row(ui, "edit", "Rename", "F2", single && can_edit && !is_space) {
            self.start_rename(item);
            ui.close();
        }
        if let Item::File(id) = item {
            if row(ui, "content_copy", "Make a copy", "", single) {
                call(&self.store, &self.inbox, ui.ctx(), Method::Post, format!("api/files/{id}/copy"), None, refresh_after);
                ui.close();
            }
            if row(ui, "download", "Download", "", single) {
                let name = self.label_of(item);
                self.download(ui.ctx(), id, &name);
                ui.close();
            }
        }
        if row(ui, "info", "Details", "", single) {
            self.show_details(ui.ctx(), item.clone());
            ui.close();
        }
        house::menu_sep(ui);
        if row(ui, "content_copy", "Copy", "Ctrl+C", !is_space) {
            self.clipboard = Some((targets.clone(), false));
            ui.close();
        }
        if row(ui, "drive_file_move", "Move to…", "", can_edit && !is_space) {
            // Start in the folder they're in now, opened up to it.
            let here = match item {
                Item::File(id) => self.file(id).map(|f| f.folder.clone()),
                Item::Folder(p) => Some(parent_of(p).to_string()),
            };
            let mut expanded = HashSet::new();
            if let Some(h) = &here {
                let mut p = h.clone();
                while !p.is_empty() {
                    expanded.insert(p.clone());
                    p = parent_of(&p).to_string();
                }
            }
            self.move_to = Some(MoveTo { items: targets.clone(), target: None, expanded });
            ui.close();
        }
        if row(ui, "content_cut", "Cut", "Ctrl+X", can_edit && !is_space) {
            self.clipboard = Some((targets.clone(), true));
            ui.close();
        }
        if let Item::Folder(f) = item
            && row(ui, "content_paste", "Paste into folder", "Ctrl+V", self.clipboard.is_some() && single && can_edit)
        {
            self.paste_into(ui.ctx(), f.clone());
            ui.close();
        }
        house::menu_sep(ui);
        if row(ui, "delete", "Move to bin", "Delete", can_edit && !is_space) {
            self.delete(ui.ctx(), targets);
            ui.close();
        }
    }

    /// Folder colour ▸ : the house's folder colours as swatches (name on hover), and No colour.
    /// Personal: only you see the colours you pick.
    pub(super) fn colour_menu(&mut self, ui: &mut Ui, path: &str, targets: &[Item]) {
        let current = self.folder(path).and_then(|f| f.colour);
        let text = RichText::new("Folder colour").size(house::BODY).color(theme::TEXT);
        let b = match crate::icons::icon("palette") {
            Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(20.0, 20.0)), text),
            None => egui::Button::new(text),
        };
        let b = b.min_size(vec2(ui.available_width(), 32.0)).right_text(egui::containers::menu::SubMenuButton::arrow_atom(Some(theme::TEXT_DIM)));
        let mut pick: Option<Option<Color32>> = None;
        egui::containers::menu::SubMenuButton::from_button(b).ui(ui, |ui| {
            house::menu(ui, 7.0 * 32.0 + 2.0 * f32::from(house::MENU_INSET));
            house::menu_inset(ui, |ui| {
                house::note(ui, "Only you see the colours you pick.");
                ui.add_space(house::S);
                for chunk in house::FOLDER_COLOURS.chunks(7) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (name, c) in chunk {
                            let (r, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                            let p = ui.painter();
                            p.circle_filled(r.center(), 11.0, *c);
                            if current == Some(*c) {
                                p.circle_stroke(r.center(), 14.0, Stroke::new(2.0, theme::TEXT));
                            } else if resp.hovered() {
                                p.circle_stroke(r.center(), 14.0, Stroke::new(2.0, theme::DIVIDER));
                            }
                            if resp.on_hover_text(*name).clicked() {
                                pick = Some(Some(*c));
                            }
                        }
                    });
                }
                ui.add_space(house::S);
            });
            if house::menu_row(ui, Some(""), "No colour", "", current.is_some()) {
                pick = Some(None);
            }
        });
        if let Some(choice) = pick {
            let colour = choice.map(house::hex).unwrap_or_default();
            for t in targets {
                if let Item::Folder(p) = t {
                    call(
                        &self.store,
                        &self.inbox,
                        ui.ctx(),
                        Method::Post,
                        format!("api/folders/colour?path={}", enc(p)),
                        Some(format!("{{\"colour\":\"{colour}\"}}").into_bytes()),
                        refresh_after,
                    );
                }
            }
            ui.close();
        }
    }

    /// Moves items to the bin; Ctrl+Z or the note's Undo puts them back (undo.rs). Something of
    /// someone else's goes to their bin, which only they can restore from: no Undo for that.
    pub(super) fn delete(&mut self, ctx: &egui::Context, items: Vec<Item>) {
        let label = if items.len() == 1 { "Moved to bin".to_string() } else { format!("{} items moved to bin", items.len()) };
        let group = self.undo.start(label, items.len(), true);
        let mine = if self.accounts { self.space.clone() } else { String::new() };
        for item in items {
            let (url, original) = match &item {
                Item::File(id) => (format!("api/files/{id}"), self.file(id).map(|f| join(&f.folder, &f.name)).unwrap_or_default()),
                Item::Folder(p) => (format!("api/folders?path={}", enc(p)), p.clone()),
            };
            let mine = mine.clone();
            call_any(&self.store, &self.inbox, ctx, Method::Delete, url, None, move |r| {
                Reply::Did(
                    group,
                    r.map(|b| {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        let bin = v["bin"].as_str().filter(|_| v["space"].as_str() == Some(mine.as_str()) && !original.is_empty())?;
                        Some(undo::Step::Binned { bin: bin.to_string(), original })
                    })
                    .map_err(|e| e.1),
                )
            });
        }
        self.selected.clear();
    }

    pub(super) fn paste_into(&mut self, ctx: &egui::Context, folder: String) {
        if let Some((items, cut)) = self.clipboard.clone() {
            if cut {
                self.clipboard = None;
            }
            self.send_to(ctx, items, folder, cut);
        }
    }

    /// Copies or moves items into a folder; a move into someone else's folder asks first.
    pub(super) fn send_to(&mut self, ctx: &egui::Context, items: Vec<Item>, folder: String, cut: bool) {
        let items: Vec<Item> = items.into_iter().filter(|i| !matches!(i, Item::Folder(p) if under(&folder, p))).collect();
        if items.is_empty() {
            return;
        }
        if cut && self.accounts {
            let to_owner = space_of(&folder).to_string();
            let owner_of = |i: &Item| match i {
                Item::File(id) => self.file(id).map(|f| f.owner.clone()).unwrap_or_default(),
                Item::Folder(p) => space_of(p).to_string(),
            };
            if items.iter().any(|i| owner_of(i) != to_owner) {
                self.pending_move = Some(PendingMove { items, to: folder, owner: to_owner });
                return;
            }
        }
        self.really_send(ctx, items, &folder, cut);
    }

    pub(super) fn really_send(&mut self, ctx: &egui::Context, items: Vec<Item>, folder: &str, cut: bool) {
        if cut {
            return self.move_items(ctx, items, folder);
        }
        for item in items {
            let url = match (&item, cut) {
                (Item::File(id), true) => format!("api/files/{id}/move?folder={}", enc(folder)),
                (Item::File(id), false) => format!("api/files/{id}/copy?folder={}", enc(folder)),
                (Item::Folder(p), true) => format!("api/folders/move?path={}&to={}", enc(p), enc(folder)),
                (Item::Folder(p), false) => format!("api/folders/copy?path={}&to={}", enc(p), enc(folder)),
            };
            call(&self.store, &self.inbox, ctx, Method::Post, url, None, refresh_after);
        }
    }

    /// Moves items into `folder` as one action that Ctrl+Z or the note's Undo can undo (undo.rs).
    fn move_items(&mut self, ctx: &egui::Context, items: Vec<Item>, folder: &str) {
        let place = self.folder_label(folder);
        let label = if items.len() == 1 { format!("Moved to {place}") } else { format!("{} items moved to {place}", items.len()) };
        let group = self.undo.start(label, items.len(), true);
        for item in items {
            let to = folder.to_string();
            let (url, then): (String, undo::Then) = match item {
                Item::File(id) => {
                    let was = self.file(&id).map(|f| join(&f.folder, &f.name)).unwrap_or_default();
                    let url = format!("api/files/{id}/move?folder={}", enc(folder));
                    (
                        url,
                        Box::new(move |v: &Json| {
                            let now = join(&to, v["name"].as_str()?);
                            let id = v["id"].as_str().map_or(id, str::to_string);
                            (!was.is_empty() && now != was).then_some(undo::Step::File { id, was, now })
                        }),
                    )
                }
                Item::Folder(p) => {
                    let url = format!("api/folders/move?path={}&to={}", enc(&p), enc(folder));
                    (
                        url,
                        Box::new(move |v: &Json| {
                            let now = v["path"].as_str()?.to_string();
                            (now != p).then_some(undo::Step::Folder { was: p, now })
                        }),
                    )
                }
            };
            call_any(&self.store, &self.inbox, ctx, Method::Post, url, None, move |r| {
                Reply::Did(group, r.map(|b| then(&serde_json::from_slice(&b).unwrap_or_default())).map_err(|e| e.1))
            });
        }
    }

    /// Move to…: a folder tree (your folders, folders shared with you that you can edit, and for
    /// the server's owner everyone's), pick one, Move here.
    pub(super) fn move_to_ui(&mut self, ctx: &egui::Context) {
        let Some(mv) = self.move_to.as_ref() else { return };
        let (items, mut target, mut expanded) = (mv.items.clone(), mv.target.clone(), mv.expanded.clone());
        let what = if items.len() == 1 {
            let label = match &items[0] {
                Item::Folder(f) => name_of(f).to_string(),
                Item::File(id) => self.files.iter().find(|f| f.id == *id).map_or_else(String::new, |f| f.name.clone()),
            };
            format!("Move \u{201c}{label}\u{201d}")
        } else {
            format!("Move {} items", items.len())
        };
        // Where things can go: folders you can edit, not into a folder being moved.
        let moving: Vec<String> = items.iter().filter_map(|i| if let Item::Folder(p) = i { Some(p.clone()) } else { None }).collect();
        let ok = |path: &str, role: &str| may_edit(role) && !moving.iter().any(|m| under(path, m));
        let mut tops: Vec<(String, String)> = vec![(self.space.clone(), if self.accounts { "My files".into() } else { "All files".into() })];
        for t in &self.shared_folders {
            if self.folder(t).is_some_and(|f| may_edit(&f.role)) {
                tops.push((t.clone(), format!("{} (shared by {})", name_of(t), self.person(space_of(t)))));
            }
        }
        if self.accounts && self.admin {
            for f in self.folders.iter().filter(|f| !f.path.contains('/') && f.path != self.space && !f.path.is_empty()) {
                tops.push((f.path.clone(), format!("{}'s files", self.person(&f.path))));
            }
        }
        let folders = self.folders.clone();
        let mut close = false;
        let mut choice = None;
        house::dialog(ctx, "home_move_to", 460.0, &what, Some(&mut close), |ui| {
            house::note(ui, "Pick the folder to move to.");
            ui.add_space(house::S);
            egui::Frame::NONE.stroke(Stroke::new(1.0, theme::DIVIDER)).corner_radius(8).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(320.0).min_scrolled_height(320.0).auto_shrink([false, false]).show(ui, |ui| {
                    fn rows(ui: &mut Ui, t: &mut PickTree, path: &str, label: &str, depth: usize) {
                        let role = t.folders.iter().find(|f| f.path == path).map_or("owner", |f| f.role.as_str());
                        let allowed = (t.ok)(path, role);
                        let mut kids: Vec<&FolderRow> = t.folders.iter().filter(|f| parent_of(&f.path) == path && f.path != path).collect();
                        kids.sort_by_key(|f| name_of(&f.path).to_lowercase());
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                        let p = ui.painter().clone();
                        let picked = t.target.as_deref() == Some(path);
                        if picked {
                            p.rect_filled(r, 6, theme::SHARE);
                        } else if resp.hovered() && allowed {
                            p.rect_filled(r, 6, theme::MENU_HOVER);
                        }
                        let x = r.left() + 4.0 + depth as f32 * 18.0;
                        let arrow = Rect::from_center_size(egui::pos2(x + 10.0, r.center().y), vec2(20.0, 20.0));
                        let open = t.expanded.contains(path);
                        let mut toggled = false;
                        if !kids.is_empty() {
                            if let Some(img) = crate::icons::icon(if open { "expand_more" } else { "chevron_right" }) {
                                img.paint_at(ui, arrow);
                            }
                            if resp.clicked() && ui.input(|i| i.pointer.interact_pos().is_some_and(|q| arrow.expand(4.0).contains(q))) {
                                toggled = true;
                                if open {
                                    t.expanded.remove(path);
                                } else {
                                    t.expanded.insert(path.to_string());
                                }
                            }
                        }
                        let f = t.folders.iter().find(|f| f.path == path).cloned().unwrap_or_default();
                        folder_icon(ui, &f, Rect::from_center_size(egui::pos2(x + 34.0, r.center().y), vec2(20.0, 20.0)));
                        p.text(
                            egui::pos2(x + 52.0, r.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(14.0),
                            if allowed { theme::TEXT } else { theme::DIVIDER },
                        );
                        if resp.clicked() && !toggled && allowed {
                            *t.target = Some(path.to_string());
                        }
                        if resp.double_clicked() && !kids.is_empty() {
                            t.expanded.insert(path.to_string());
                        }
                        if t.expanded.contains(path) {
                            for k in kids {
                                rows(ui, t, &k.path, name_of(&k.path), depth + 1);
                            }
                        }
                    }
                    for (path, label) in &tops {
                        rows(ui, &mut PickTree { folders: &folders, ok: &ok, target: &mut target, expanded: &mut expanded }, path, label, 0);
                    }
                });
            });
            choice = house::actions_in(ui, &["Cancel", "Move here"], ACCENT);
        });
        let mv = self.move_to.as_mut().expect("open");
        mv.target = target.clone();
        mv.expanded = expanded;
        if close || choice == Some(0) || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.move_to = None;
        } else if choice == Some(1) {
            match target {
                Some(to) => {
                    let items = self.move_to.take().map(|m| m.items).unwrap_or_default();
                    self.send_to(ctx, items, to, true);
                }
                None => self.error = Some("Pick a folder to move to first.".into()),
            }
        }
    }

    pub(super) fn show_details(&mut self, ctx: &egui::Context, item: Item) {
        self.details = Some((item.clone(), None));
        if self.accounts && !matches!(&item, Item::Folder(p) if !p.contains('/')) {
            let url = match &item {
                Item::File(id) => format!("api/files/{id}/sharing"),
                Item::Folder(p) => format!("api/folders/sharing?path={}", enc(p)),
            };
            call(&self.store, &self.inbox, ctx, Method::Get, url, None, move |b| Reply::Access(item, serde_json::from_slice(&b).unwrap_or_default()));
        }
    }

    /// Where a path is, in words: "My files › Budgets", "Ada's files › Team".
    pub(super) fn place(&self, folder: &str) -> String {
        let top = space_of(folder);
        let mut parts = vec![if !self.accounts {
            "All files".to_string()
        } else if top == self.space {
            "My files".into()
        } else {
            format!("{}'s files", self.person(top))
        }];
        parts.extend(folder.split('/').skip(usize::from(self.accounts)).filter(|p| !p.is_empty()).map(str::to_string));
        parts.join(" › ")
    }

    /// Details: what it is, where, whose, how big, when it changed and by whom, who can open it.
    pub(super) fn details_ui(&mut self, ctx: &egui::Context) {
        let Some((item, access)) = self.details.clone() else { return };
        let mut rows: Vec<(&str, String)> = Vec::new();
        let title;
        match &item {
            Item::File(id) => {
                let Some(f) = self.file(id).cloned() else {
                    self.details = None;
                    return;
                };
                title = f.name.clone();
                let ext = f.name.rsplit_once('.').map_or("xlsx".to_string(), |(_, e)| e.to_lowercase());
                rows.push(("Type", format!("Spreadsheet (.{ext})")));
                rows.push(("Location", self.place(&f.folder)));
                if self.accounts {
                    rows.push(("Owner", if f.owner == self.space { "You".into() } else { self.person(&f.owner) }));
                }
                rows.push(("Size", size_text(f.size)));
                let by = if f.changed_by.is_empty() {
                    String::new()
                } else if f.changed_by == self.space {
                    " by you".into()
                } else {
                    format!(" by {}", self.person(&f.changed_by))
                };
                rows.push(("Modified", format!("{}{by}", ago(f.modified))));
                if let Some(t) = f.opened {
                    rows.push(("Opened by you", ago(t)));
                }
                if self.accounts {
                    rows.push((
                        "Your access",
                        match f.role.as_str() {
                            "owner" => "Owner",
                            "edit" => "Can edit",
                            "comment" => "Can comment",
                            _ => "Can view",
                        }
                        .into(),
                    ));
                }
            }
            Item::Folder(p) => {
                let Some(f) = self.folder(p).cloned() else {
                    self.details = None;
                    return;
                };
                title = self.folder_label(p);
                rows.push(("Type", "Folder".into()));
                if p.contains('/') || !self.accounts {
                    rows.push(("Location", self.place(parent_of(p))));
                }
                if self.accounts {
                    rows.push(("Owner", if f.owner == self.space { "You".into() } else { self.person(&f.owner) }));
                }
                let files = self.files.iter().filter(|x| under(&x.folder, p)).count();
                let folders = self.folders.iter().filter(|x| x.path != *p && under(&x.path, p)).count();
                let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
                rows.push(("Contains", format!("{}, {}", plural(files, "spreadsheet"), plural(folders, "folder"))));
                let (newest, size) = self.folder_stats(p);
                rows.push(("Size", size_text(size)));
                if newest > 0 {
                    rows.push(("Last change inside", ago(newest)));
                }
                if let Some(c) = f.colour {
                    let name = house::FOLDER_COLOURS.iter().find(|(_, x)| *x == c).map_or("", |(n, _)| *n);
                    rows.push(("Colour (only you see it)", name.to_string()));
                }
            }
        }
        // Who has access: the owner, the people it's shared with, and the link setting.
        let mut access_lines: Vec<String> = Vec::new();
        if let Some(a) = &access {
            for person in a["people"].as_array().into_iter().flatten() {
                let role = match person["role"].as_str().unwrap_or("") {
                    "owner" => "owner",
                    "edit" => "can edit",
                    "comment" => "can comment",
                    _ => "can view",
                };
                access_lines.push(format!("{} ({role})", person["name"].as_str().unwrap_or("")));
            }
            match a["link"].as_str().unwrap_or("none") {
                "view" => access_lines.push("Anyone signed in with the link can view".into()),
                "comment" => access_lines.push("Anyone signed in with the link can comment".into()),
                "edit" => access_lines.push("Anyone signed in with the link can edit".into()),
                _ => {}
            }
        }
        let mut close = false;
        let mut done = false;
        let mut share = false;
        house::dialog(ctx, "home_details", 480.0, &title, Some(&mut close), |ui| {
            egui::Grid::new("home_details_grid").num_columns(2).spacing(vec2(house::L, house::S)).show(ui, |ui| {
                for (k, v) in &rows {
                    ui.label(RichText::new(*k).size(house::BODY).color(theme::TEXT_DIM));
                    ui.label(RichText::new(v).size(house::BODY).color(theme::TEXT));
                    ui.end_row();
                }
                if self.accounts && !matches!(&item, Item::Folder(p) if !p.contains('/')) {
                    ui.label(RichText::new("Who has access").size(house::BODY).color(theme::TEXT_DIM));
                    ui.vertical(|ui| {
                        if access.is_none() {
                            ui.label(RichText::new("Loading…").size(house::BODY).color(theme::TEXT_DIM));
                        }
                        for l in &access_lines {
                            ui.label(RichText::new(l).size(house::BODY).color(theme::TEXT));
                        }
                    });
                    ui.end_row();
                }
            });
            let can_share = self.accounts && self.role_of(&item) == "owner" && !matches!(&item, Item::Folder(p) if !p.contains('/'));
            let labels: &[&str] = if can_share { &["Share…", "Close"] } else { &["Close"] };
            match house::actions_in(ui, labels, ACCENT) {
                Some(0) if can_share => share = true,
                Some(_) => done = true,
                None => {}
            }
        });
        if share {
            let (folder, key) = match &item {
                Item::Folder(p) => (true, p.clone()),
                Item::File(id) => (false, id.clone()),
            };
            #[cfg(target_arch = "wasm32")]
            crate::share::share_item(ctx, folder, &key, &title);
            #[cfg(not(target_arch = "wasm32"))]
            let _ = (folder, key, title);
            self.details = None;
        } else if close || done || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.details = None;
        }
    }

    pub(super) fn pending_move_ui(&mut self, ctx: &egui::Context) {
        let Some(pm) = &self.pending_move else { return };
        let who = self.person(&pm.owner);
        let what = if pm.items.len() == 1 { format!("\u{201c}{}\u{201d}", self.label_of(&pm.items[0])) } else { format!("{} items", pm.items.len()) };
        let text = if who == "me" {
            format!("Move {what} into your own folder? You'll own it, and the people it was shared with through its old folder lose access.")
        } else {
            format!("Move {what} into {who}'s folder? {who} will own it and decide who else can open it.")
        };
        let mut close = false;
        let mut choice = None;
        house::dialog(ctx, "home_move_ask", 420.0, "Change who owns it?", Some(&mut close), |ui| {
            house::text(ui, &text);
            choice = house::actions_in(ui, &["Cancel", "Move"], ACCENT);
        });
        if close || choice == Some(0) {
            self.pending_move = None;
        } else if choice == Some(1)
            && let Some(pm) = self.pending_move.take()
        {
            self.really_send(ctx, pm.items, &pm.to, true);
        }
    }
}

/// What the "Move to" folder tree reads and changes as it draws itself, row by row.
struct PickTree<'a> {
    folders: &'a [FolderRow],
    ok: &'a dyn Fn(&str, &str) -> bool,
    target: &'a mut Option<String>,
    expanded: &'a mut HashSet<String>,
}
