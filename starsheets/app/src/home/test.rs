//! The home screen's side of the test hook (`crate::testhook`, app/TESTING.md): what it shows, as
//! JSON, and the few things a test can do on it without clicking.

use serde_json::json;

use super::{Home, Json, View, name_of};

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
            "view": view,
            "loading": self.loading,
            "search": self.search,
            "error": self.error,
            "folders": folders.iter().map(|f| json!({ "path": f.path, "name": name_of(&f.path) })).collect::<Vec<_>>(),
            "files": files
                .iter()
                .map(|f| json!({ "id": f.id, "name": f.name, "folder": f.folder, "starred": f.starred, "kind": super::items::file_kind(&f.name) }))
                .collect::<Vec<_>>(),
        })
    }

    /// `home:folder` {path}, `home:search` {text}, `home:refresh`, `home:recent`, `home:starred`,
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
            _ => return Err(format!("unknown home screen command `{id}`")),
        }
        ctx.request_repaint();
        Ok(json!({ "done": id }))
    }
}
