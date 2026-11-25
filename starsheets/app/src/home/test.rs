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
            Some(View::Everyone(p)) => json!({ "everyone": p }),
            None => Json::Null,
        };
        json!({
            "photos": self.photos.test_state(self.photo_folder().is_some()),
            "view": view,
            "loading": self.loading,
            "search": self.search,
            "error": self.error,
            "undo": self.undo.done.iter().map(|a| a.label.clone()).collect::<Vec<_>>(),
            "note": self.note_text(),
            "folders": folders.iter().map(|f| json!({ "path": f.path, "name": name_of(&f.path) })).collect::<Vec<_>>(),
            "files": files
                .iter()
                .map(|f| json!({ "id": f.id, "name": f.name, "folder": f.folder, "starred": f.starred, "kind": super::items::file_kind(&f.name) }))
                .collect::<Vec<_>>(),
        })
    }

    /// `home:folder` {path}, `home:search` {text}, `home:refresh`, `home:move`, `home:delete`, `home:undo`, `home:recent`, `home:starred`,
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
            "home:recent" => self.navigate(View::Recent),
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
            "home:photos:tab" | "home:photos:select" | "home:photos:place:open" => pm.alb_test(ctx, &self.store, id, params)?,
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
