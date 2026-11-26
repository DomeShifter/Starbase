//! The home screen's side of the test hook (`crate::testhook`, app/TESTING.md): what it shows, as
//! JSON, and the few things a test can do on it without clicking.

use serde_json::json;

use super::{Home, Item, Json, View, name_of};

impl Home {
    /// Where the home screen is and what it lists, in the order shown.
    pub(crate) fn test_state(&self) -> Json {
        let (folders, files) = self.contents();
        let view = match &self.view {
            Some(View::Mine(p)) => json!({ "mine": p }),
            Some(View::Shared(p)) => json!({ "shared": p }),
            Some(View::Recent) => json!("recent"),
            Some(View::Starred) => json!("starred"),
            Some(View::Bin) => json!("bin"),
            Some(View::Storage) => json!("storage"),
            Some(View::Everyone(p)) => json!({ "everyone": p }),
            None => Json::Null,
        };
        json!({
            "preview": self.file_preview.as_ref().map(|p| p.test_state()),
            // The last right-click menu's rows: [{label, rect: [x0, y0, x1, y1]}].
            "menu": self.menu_rects.iter().map(|(l, r)| json!({ "label": l, "rect": [r.min.x, r.min.y, r.max.x, r.max.y] })).collect::<Vec<_>>(),
            "photos": self.photos.test_state(self.photo_folder().is_some()),
            "view": view,
            "storage": self.storage.map(|(u, b, f)| json!({ "used": u, "bin": b, "free": f })),
            "loading": self.loading,
            "activity": self.activity.as_ref().map(|(_, v)| super::activity::rows(v)),
            "search": self.search,
            "filters": { "kind": self.filters.kind, "who": self.filters.who, "when": self.filters.when },
            "error": self.error,
            "undo": self.undo.done.iter().map(|a| a.label.clone()).collect::<Vec<_>>(),
            "note": self.note_text(),
            "folders": folders.iter().map(|f| json!({ "path": f.path, "name": name_of(&f.path) })).collect::<Vec<_>>(),
            "files": files
                .iter()
                .map(|f| {
                    // The tile picture: "ready" (texture loaded), "loading", "none" (plain icon) or null.
                    let thumb = self.thumbs.get(&f.id).map(|(_, t)| match t {
                        super::Thumb::Ready(_) => "ready",
                        super::Thumb::Loading => "loading",
                        super::Thumb::None => "none",
                    });
                    json!({ "id": f.id, "name": f.name, "folder": f.folder, "starred": f.starred, "kind": super::items::file_kind(&f.name), "thumb": thumb, "rect": self.tile_rects.get(&f.id).map(|r| [r.min.x, r.min.y, r.max.x, r.max.y]) })
                })
                .collect::<Vec<_>>(),
        })
    }

    /// `home:folder` {path}, `home:search` {text}, `home:filter` {kind, who, when}, `home:refresh`, `home:move`, `home:delete`, `home:undo`, `home:recent`, `home:starred`,
    /// `home:shared`, `home:bin`; `home:opens` {id} says what opening that file does (without
    /// opening it): `{"opens": "editor" | "preview" | "download", "url"}`.
    pub(crate) fn test_run(&mut self, ctx: &egui::Context, id: &str, params: &Json) -> Result<Json, String> {
        let text = |key: &str| params.get(key).and_then(Json::as_str).map(str::to_string);
        match id {
            "home:folder" => {
                let path = text("path").ok_or("home:folder needs {path}")?;
                self.go_folder(&path);
            }
            "home:search" => self.search = text("text").unwrap_or_default(),
            // {kind: folder|sheet|doc|slides|image|pdf|null, who: 0..2, when: 0..4}
            "home:filter" => {
                self.filters.kind = super::filters::KINDS.iter().map(|(k, _)| *k).find(|k| Some(*k) == text("kind").as_deref());
                let n = |k: &str| params.get(k).and_then(Json::as_u64).unwrap_or(0) as u8;
                (self.filters.who, self.filters.when) = (n("who").min(2), n("when").min(4));
            }
            "home:refresh" => self.refresh(ctx),
            // Undo (undo.rs): `home:move` {files: [id], folders: [path], to} moves as a drag
            // does, `home:delete` {files, folders} bins, `home:undo` is Ctrl+Z.
            "home:move" | "home:delete" => {
                let list = |k: &str| params.get(k).and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str).map(str::to_string);
                let items: Vec<Item> = list("files").map(Item::File).chain(list("folders").map(Item::Folder)).collect();
                if items.is_empty() {
                    return Err(format!("{id} needs {{files}} or {{folders}}"));
                }
                if id == "home:delete" {
                    self.delete(ctx, items);
                } else {
                    let to = text("to").ok_or("home:move needs {to}")?;
                    self.send_to(ctx, items, to, true);
                }
            }
            "home:undo" => self.undo(ctx),
            // `home:download` {files: [id], folders: [path]}: Download as the menu does it (one
            // file as itself, else one zip); says which address it fetches.
            "home:download" => {
                let list = |k: &str| params.get(k).and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str).map(str::to_string);
                let items: Vec<Item> = list("files").map(Item::File).chain(list("folders").map(Item::Folder)).collect();
                if items.is_empty() {
                    return Err("home:download needs {files} or {folders}".into());
                }
                let url = if let [Item::File(id)] = items.as_slice() { format!("api/files/{id}") } else { Self::zip_url(&items) };
                self.download_items(ctx, &items);
                return Ok(json!({ "url": url }));
            }
            "home:opens" => {
                let id = text("id").ok_or("home:opens needs {id}")?;
                let f = self.file(&id).ok_or("no such file listed")?;
                let (opens, url) = match super::items::opens_as(&f.name) {
                    super::items::Opens::Editor => ("editor", crate::files::file_url(&id, &f.name)),
                    super::items::Opens::Preview => ("preview", Self::preview_url(&id)),
                    super::items::Opens::Download => ("download", format!("api/files/{id}")),
                };
                return Ok(json!({ "opens": opens, "url": url }));
            }
            // The file preview (filepreview.rs): `home:preview` {id} opens it, `home:preview:close`.
            "home:preview" => {
                let id = text("id").ok_or("home:preview needs {id}")?;
                self.file(&id).ok_or("no such file listed")?;
                self.preview(ctx, &id);
            }
            "home:preview:close" => self.file_preview = None,
            "home:recent" => self.navigate(View::Recent),
            "home:storage" => {
                self.navigate(View::Storage);
                self.load_storage(ctx);
            }
            "home:starred" => self.navigate(View::Starred),
            "home:shared" => self.navigate(View::Shared(None)),
            "home:bin" => {
                self.navigate(View::Bin);
                self.load_bin(ctx);
            }
            id if id.starts_with("home:photos") => return self.photos_test(ctx, id, params),
            _ => return Err(format!("unknown home screen command `{id}`")),
        }
        ctx.request_repaint();
        Ok(json!({ "done": id }))
    }

    /// `home:photos` {on}, `home:photos:edit` {open} (on the picture the viewer shows),
    /// `home:photos:edit:<what>` (photo_edit.rs `test_run`), `home:photos:enable`, `home:photos:sub` {on}, `home:photos:order`
    /// {newest}, `home:photos:open` {index}, `home:photos:key` {key: next, prev, close, zoomin,
    /// zoomout, fit, play, pause}, `home:photos:slideshow` {every, shuffle, play}.
    fn photos_test(&mut self, ctx: &egui::Context, id: &str, params: &Json) -> Result<Json, String> {
        let flag = |k: &str, d: bool| params.get(k).and_then(Json::as_bool).unwrap_or(d);
        let now = ctx.input(|i| i.time);
        let folder = self.photo_folder();
        let pm = &mut self.photos;
        match id {
            "home:photos" => pm.set_on(&self.store, flag("on", true)),
            "home:photos:enable" => {
                let f = folder.ok_or("no folder is shown as photos")?;
                pm.switch(ctx, &self.store, &f, true);
            }
            "home:photos:sub" | "home:photos:order" => {
                if id == "home:photos:sub" {
                    pm.sub = flag("on", true);
                } else {
                    pm.newest_first = flag("newest", true);
                }
                if let Some(f) = folder {
                    pm.pics.clear();
                    pm.list(ctx, &self.store, &f);
                }
            }
            "home:photos:edit" => {
                if params.get("open").and_then(Json::as_bool).unwrap_or(true) {
                    if pm.viewer.is_none() {
                        return Err("the viewer isn't open".into());
                    }
                    super::photo_edit::open(pm, ctx, &self.store);
                } else {
                    pm.editor = None;
                }
            }
            id if id.starts_with("home:photos:edit:") => {
                let ed = pm.editor.as_mut().ok_or("the editor isn't open")?;
                ed.test_run(ctx, &self.store, &id["home:photos:edit:".len()..], params)?;
            }
            "home:photos:tab" | "home:photos:dups:keep" | "home:photos:select" | "home:photos:place:open" => {
                pm.alb_test(ctx, &self.store, id, params)?
            }
            "home:photos:click" | "home:photos:menu" | "home:photos:bar" | "home:photos:addto" | "home:photos:picker" | "home:photos:note:open" => {
                pm.pick_test(ctx, &self.store, id, params)?
            }
            id if id.starts_with("home:photos:album:") => pm.alb_test(ctx, &self.store, id, params)?,
            "home:photos:open" => {
                let i = params.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
                if i >= pm.pics.len() {
                    return Err("no such picture".into());
                }
                pm.viewer = Some(super::viewer::Viewer::new(i, pm.pics.len()));
            }
            "home:photos:key" | "home:photos:slideshow" => {
                let v = pm.viewer.as_mut().ok_or("the viewer isn't open")?;
                if id == "home:photos:slideshow" {
                    if let Some(e) = params.get("every").and_then(Json::as_f64) {
                        v.every = e;
                    }
                    if let Some(sh) = params.get("shuffle").and_then(Json::as_bool) {
                        v.set_shuffle(sh, (now * 1000.0) as u64);
                    }
                    if flag("play", true) {
                        v.play(now);
                    } else {
                        v.playing = false;
                    }
                } else {
                    match params.get("key").and_then(Json::as_str).unwrap_or("") {
                        "next" => v.step(1),
                        "prev" => v.step(-1),
                        "zoomin" => v.zoom = (v.zoom * 1.5).min(8.0),
                        "zoomout" => v.zoom = (v.zoom / 1.5).max(1.0),
                        "fit" => v.zoom = 1.0,
                        "play" => v.play(now),
                        "pause" => v.playing = false,
                        "close" => pm.viewer = None,
                        k => return Err(format!("unknown key `{k}`")),
                    }
                }
            }
            _ => return Err(format!("unknown photos command `{id}`")),
        }
        ctx.request_repaint();
        Ok(serde_json::json!({ "done": id }))
    }
}
