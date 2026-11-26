//! The home screen (StarSheets opened without a file), like Google Sheets' and Drive's: a folder
//! tree on the left (My files, Shared with me, Recent, Starred, Bin, and for the server's owner
//! Everyone's files), and the folders and spreadsheets of the chosen place on the right, as cards
//! with a preview picture or as a list. Everything is organised here: new spreadsheets and
//! folders, uploading (the New button, or dropping files on the page), opening, renaming, starring,
//! sharing (files and folders), copying, moving (drag onto a folder, the tree or the path; or cut
//! and paste) and the bin. Moving something into another person's folder makes it theirs, so the
//! app asks first.
//!
//! Server: crates/server (GET /api/files lists what you may see, with owner, role, stars and when
//! you opened each file; /api/files/ID/thumb is the preview picture).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use egui::{Align2, Color32, FontId, Key, Rect, RichText, Sense, Stroke, Ui, vec2};
use serde_json::Value as Json;

use crate::house;
use crate::store::{Backend, Method, enc};
use crate::theme;

mod actions;
mod bin;
mod filepreview;
mod filters;
mod input;
mod items;
mod photo_edit;
mod photos;
mod sidebar;
mod templates;
#[cfg(target_arch = "wasm32")]
mod test;
pub(crate) mod textpreview;
mod undo;
mod viewer;

const LEFT: f32 = 256.0;
const TOP: f32 = 64.0;
const CARD_W: f32 = 216.0;
const CARD_H: f32 = 196.0;
const PIC_H: f32 = 144.0;
const CHIP_H: f32 = 48.0;
const ROW: f32 = 40.0;
/// Starbase Drive's accent: Google Drive's blue (drop targets, dialog buttons), not Sheets' green.
const ACCENT: Color32 = house::BLUE;
/// Preview pictures fetched at once.
const THUMBS_AT_ONCE: usize = 4;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Item {
    File(String),
    Folder(String),
}

#[derive(Clone, Debug, Default)]
struct FileRow {
    id: String,
    name: String,
    folder: String,
    modified: i64,
    size: u64,
    version: String,
    owner: String,
    role: String,
    starred: bool,
    shared: bool,
    opened: Option<i64>,
    /// Who saved the last change (username), when known.
    changed_by: String,
}

#[derive(Clone, Debug, Default)]
struct FolderRow {
    path: String,
    owner: String,
    role: String,
    starred: bool,
    shared: bool,
    /// The colour you gave it (house::FOLDER_COLOURS), if any.
    colour: Option<Color32>,
}

#[derive(Clone, Debug)]
struct BinRow {
    name: String,
    original: String,
    deleted: i64,
    folder: bool,
}

#[derive(Clone, Debug, Default)]
struct Person {
    name: String,
}

/// Where the right-hand side is.
#[derive(Clone, PartialEq, Debug)]
enum View {
    /// A folder in my own space (the path includes the space).
    Mine(String),
    /// Shared with me: the tops (None) or inside a shared folder.
    Shared(Option<String>),
    Recent,
    Starred,
    Bin,
    /// The server owner's view of every folder ("" = everyone's folders).
    Everyone(String),
}

enum Reply {
    Listed(Json),
    Bin(Vec<BinRow>),
    Refresh,
    NewFolder(String),
    Thumb(String, String, Option<egui::ColorImage>),
    Download(String, Vec<u8>),
    /// Who has access to the item in Details.
    Access(Item, Json),
    /// One request of an undoable action answered: what it did (None: nothing), or why not.
    Did(u64, Result<Option<undo::Step>, String>),
    /// One request of an undo answered.
    Undone(Result<(), String>),
    /// The files whose text matches a search: (the search, [(file id, snippet)]).
    Found(String, Vec<(String, String)>),
    Error(String),
}

enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    None,
}

/// The Move to… picker: what's being moved, the folder picked, and which tree rows are open.
struct MoveTo {
    items: Vec<Item>,
    target: Option<String>,
    expanded: HashSet<String>,
}

/// A move that hands things to someone else, waiting for "Move" in the question.
struct PendingMove {
    items: Vec<Item>,
    to: String,
    owner: String,
}

#[derive(Default)]
pub struct Home {
    /// Where the files are: the server, or a folder on disk in the desktop app (store.rs).
    store: Backend,
    /// Shown instead of a spreadsheet (the page was opened without ?file=, ?new or ?folder=).
    #[cfg(target_arch = "wasm32")]
    pub active: bool,
    booted: bool,
    inbox: Rc<RefCell<Vec<Reply>>>,
    loading: bool,
    files: Vec<FileRow>,
    folders: Vec<FolderRow>,
    bin: Vec<BinRow>,
    /// My own folder ("" without accounts), whether I'm the server's owner, the tops of "Shared
    /// with me", and everyone's names.
    space: String,
    admin: bool,
    accounts: bool,
    shared_folders: Vec<String>,
    shared_files: Vec<String>,
    people: HashMap<String, Person>,
    view: Option<View>,
    /// Where you were before (Back) and where Back came from (Forward).
    back: Vec<View>,
    forward: Vec<View>,
    expanded: HashSet<String>,
    list: bool,
    /// Sort: by (0 name, 1 date modified, 2 size), Z–A / newest / largest first, and whether
    /// folders are mixed in with files instead of kept at the top.
    sort_by: u8,
    descending: bool,
    mixed: bool,
    /// Details: the item shown, and who has access to it (from the server) once known.
    details: Option<(Item, Option<Json>)>,
    search: String,
    /// Files whose text holds the search words (id → snippet), for the search in `found_for`.
    found: HashMap<String, String>,
    found_for: String,
    /// The search last sent to the server for text matches.
    asked: String,
    /// The chips under the search box (filters.rs).
    filters: filters::Filters,
    selected: Vec<Item>,
    anchor: Option<Item>,
    clipboard: Option<(Vec<Item>, bool)>,
    renaming: Option<(Item, String, bool)>,
    /// A folder just made with New folder: shown first until it has been named (then the list
    /// sorts it), as in Drive.
    just_made: Option<String>,
    dragging: Option<Vec<Item>>,
    confirm_purge: Option<Option<String>>,
    pending_move: Option<PendingMove>,
    /// Move to…: the folder picker, open for these items.
    move_to: Option<MoveTo>,
    thumbs: HashMap<String, (String, Thumb)>,
    fetching: usize,
    /// "View as photos" (photos.rs, viewer.rs).
    photos: photos::PhotoMode,
    /// The file preview over the Drive (filepreview.rs), when open.
    file_preview: Option<filepreview::FilePreview>,
    /// Where each file was last drawn (id → screen rect), for the test hook's real clicks.
    tile_rects: HashMap<String, egui::Rect>,
    /// The rows of the last right-click menu drawn (label, screen rect), for the test hook's real clicks.
    menu_rects: Vec<(String, egui::Rect)>,
    /// What Ctrl+Z undoes, and the "Moved to … Undo" note (undo.rs).
    undo: undo::Undo,
    error: Option<String>,
    /// A folder to open once the list arrives (?dir= in the address).
    open_dir: Option<String>,
    /// The desktop app's "Connect to a server…" (remote.rs).
    #[cfg(not(target_arch = "wasm32"))]
    connect: crate::remote::Connect,
    /// When the list was last asked for (the desktop asks again now and then: files are saved by
    /// the spreadsheet windows, other programs and other people).
    #[cfg(not(target_arch = "wasm32"))]
    listed_at: f64,
}

/// How often the desktop's Drive lists the files again (ms).
#[cfg(not(target_arch = "wasm32"))]
const RELIST_MS: f64 = 10_000.0;

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") }
}

fn space_of(path: &str) -> &str {
    path.split('/').next().unwrap_or("")
}

fn under(path: &str, top: &str) -> bool {
    path == top || path.starts_with(&format!("{top}/"))
}

#[cfg(target_arch = "wasm32")]
fn query(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(name)
}

/// Whether this page should be the home screen: no file, new spreadsheet or folder asked for.
#[cfg(target_arch = "wasm32")]
pub fn wanted() -> bool {
    query("file").is_none() && query("new").is_none() && query("folder").is_none()
}

fn call(
    store: &Backend,
    inbox: &Rc<RefCell<Vec<Reply>>>,
    ctx: &egui::Context,
    method: Method,
    url: String,
    body: Option<Vec<u8>>,
    then: impl FnOnce(Vec<u8>) -> Reply + 'static,
) {
    call_any(store, inbox, ctx, method, url, body, move |r| match r {
        Ok(bytes) => then(bytes),
        Err((_, e)) => Reply::Error(e),
    });
}

/// As [`call`], but `then` also gets a refusal: (status, the server's words). 401 asks to sign
/// in and comes back with empty words.
fn call_any(
    store: &Backend,
    inbox: &Rc<RefCell<Vec<Reply>>>,
    ctx: &egui::Context,
    method: Method,
    url: String,
    body: Option<Vec<u8>>,
    then: impl FnOnce(Result<Vec<u8>, (u16, String)>) -> Reply + 'static,
) {
    let (inbox2, ctx2) = (inbox.clone(), ctx.clone());
    store.call(
        ctx,
        method,
        &url,
        body,
        Box::new(move |answer| {
            let reply = match answer {
                Ok((s, bytes)) if s < 400 => then(Ok(bytes)),
                Ok((401, _)) => {
                    #[cfg(target_arch = "wasm32")]
                    crate::account::ask_sign_in();
                    then(Err((401, String::new())))
                }
                Ok((s, bytes)) => {
                    let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
                    then(Err((s, v["error"].as_str().map_or(format!("The server said {s}."), str::to_string))))
                }
                Err(e) => then(Err((0, e))),
            };
            inbox2.borrow_mut().push(reply);
            ctx2.request_repaint();
        }),
    );
}

fn refresh_after(_: Vec<u8>) -> Reply {
    Reply::Refresh
}

fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let px = &buf[..info.buffer_size()];
    let size = [info.width as usize, info.height as usize];
    match info.color_type {
        png::ColorType::Rgb => Some(egui::ColorImage::from_rgb(size, px)),
        png::ColorType::Rgba => Some(egui::ColorImage::from_rgba_unmultiplied(size, px)),
        _ => None,
    }
}

fn ago(unix: i64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        crate::files::date_text(unix)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::localstore::date_text(unix)
    }
}

impl Home {
    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Home {
        Home { active: wanted(), ..Default::default() }
    }

    /// The home screen over another store (the desktop app's local folder).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn with_store(store: Backend) -> Home {
        Home { store, ..Default::default() }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.listed_at = gridcraft_ui_egui::monotonic_ms();
        }
        self.loading = true;
        call(&self.store, &self.inbox, ctx, Method::Get, "api/files".into(), None, |b| Reply::Listed(serde_json::from_slice(&b).unwrap_or_default()));
        if self.view == Some(View::Bin) {
            self.load_bin(ctx);
        }
    }

    /// Search also looks inside files: asks the server which files' text holds the words (once
    /// per search; an answer for an older search is dropped when it arrives).
    fn ask_text_matches(&mut self, ctx: &egui::Context) {
        let q = self.search.trim().to_string();
        if q == self.asked {
            return;
        }
        self.asked = q.clone();
        if q.chars().count() < 2 {
            self.found.clear();
            self.found_for.clear();
            return;
        }
        let url = format!("api/content-search?q={}", enc(&q));
        let q2 = q.clone();
        call_any(&self.store, &self.inbox, ctx, Method::Get, url, None, move |r| {
            let hits = r.ok().and_then(|b| serde_json::from_slice::<Json>(&b).ok()).map(|v| {
                v["hits"]
                    .as_array()
                    .map(|a| {
                        a.iter().filter_map(|h| Some((h["id"].as_str()?.to_string(), h["snippet"].as_str().unwrap_or("").to_string()))).collect()
                    })
                    .unwrap_or_default()
            });
            Reply::Found(q2, hits.unwrap_or_default())
        });
    }

    fn load_bin(&mut self, ctx: &egui::Context) {
        call(&self.store, &self.inbox, ctx, Method::Get, "api/bin".into(), None, |b| {
            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
            Reply::Bin(
                v.as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| BinRow {
                        name: x["name"].as_str().unwrap_or("").into(),
                        original: x["original"].as_str().unwrap_or("").into(),
                        deleted: x["deleted"].as_i64().unwrap_or(0),
                        folder: x["folder"].as_bool().unwrap_or(false),
                    })
                    .collect(),
            )
        });
    }

    fn take_list(&mut self, v: Json) {
        self.loading = false;
        let s = |x: &Json, k: &str| x[k].as_str().unwrap_or("").to_string();
        self.accounts = v.get("space").is_some();
        self.space = s(&v, "space");
        self.admin = v["admin"].as_bool().unwrap_or(!self.accounts);
        self.files = v["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| FileRow {
                id: s(f, "id"),
                name: s(f, "name"),
                folder: s(f, "folder"),
                modified: f["modified"].as_i64().unwrap_or(0),
                size: f["size"].as_u64().unwrap_or(0),
                version: s(f, "version"),
                owner: s(f, "owner"),
                role: f["role"].as_str().unwrap_or("owner").into(),
                starred: f["starred"].as_bool().unwrap_or(false),
                shared: f["shared"].as_bool().unwrap_or(false),
                opened: f["opened"].as_i64(),
                changed_by: s(f, "changed_by"),
            })
            .collect();
        let info = v["folder_info"].as_object().cloned().unwrap_or_default();
        self.folders = v["folders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str())
            .map(|p| {
                let i = info.get(p).cloned().unwrap_or_default();
                FolderRow {
                    path: p.into(),
                    owner: s(&i, "owner"),
                    role: i["role"].as_str().unwrap_or("owner").into(),
                    starred: i["starred"].as_bool().unwrap_or(false),
                    shared: i["shared"].as_bool().unwrap_or(false),
                    colour: house::folder_colour(i["colour"].as_str().unwrap_or("")),
                }
            })
            .collect();
        let list = |k: &str| v["shared"][k].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>();
        self.shared_folders = list("folders");
        self.shared_files = list("files");
        self.people = v["people"].as_array().into_iter().flatten().map(|p| (s(p, "username"), Person { name: s(p, "name") })).collect();
        if self.view.is_none() {
            self.view = Some(View::Mine(self.space.clone()));
            self.expanded.insert(format!("mine:{}", self.space));
        }
        if let Some(dir) = self.open_dir.take() {
            self.go_folder(&dir);
        }
        // Forget pictures of files that went away. A document's picture is sent by StarDocs just
        // after it saves, so one not there yet is asked for again at the next listing.
        let ids: HashSet<&String> = self.files.iter().map(|f| &f.id).collect();
        let docs: HashSet<&String> = self.files.iter().filter(|f| crate::store::is_doc(&f.name)).map(|f| &f.id).collect();
        self.thumbs.retain(|k, (_, t)| ids.contains(k) && !(docs.contains(k) && matches!(t, Thumb::None)));
    }

    fn person(&self, username: &str) -> String {
        if username == self.space && self.accounts {
            return "me".into();
        }
        self.people.get(username).map_or_else(|| username.to_string(), |p| p.name.clone())
    }

    fn folder(&self, path: &str) -> Option<&FolderRow> {
        self.folders.iter().find(|f| f.path == path)
    }

    fn file(&self, id: &str) -> Option<&FileRow> {
        self.files.iter().find(|f| f.id == id)
    }

    /// The address of file `id`'s preview page (crates/server/src/preview.rs), relative to the page.
    #[cfg(target_arch = "wasm32")]
    fn preview_url(id: &str) -> String {
        format!("api/files/{id}/preview")
    }

    /// The files the preview's arrows go through: those of the view shown that don't open in an
    /// editor, in the order shown.
    fn previewable(&self) -> Vec<String> {
        self.contents().1.into_iter().filter(|f| items::opens_as(&f.name) != items::Opens::Editor).map(|f| f.id).collect()
    }

    /// Opens Drive's preview of file `id` over the file list (filepreview.rs).
    fn preview(&mut self, ctx: &egui::Context, id: &str) {
        let Some(f) = self.file(id) else { return };
        let (name, size, modified, owner) = (f.name.clone(), f.size, f.modified, f.owner.clone());
        let owner = self.person(&owner);
        let list = self.previewable();
        let at = (list.iter().position(|x| x == id).unwrap_or(0), list.len().max(1));
        self.file_preview = Some(filepreview::FilePreview::open(ctx, &self.store, id, &name, size, modified, owner, at));
    }

    /// One frame of the preview, if open; whether it was (it takes the keys then).
    fn file_preview_ui(&mut self, ctx: &egui::Context) -> bool {
        let Some(p) = self.file_preview.as_mut() else { return false };
        match p.ui(ctx) {
            Some(filepreview::Act::Close) => self.file_preview = None,
            Some(filepreview::Act::Step(by)) => {
                let (cur, info) = (p.id.clone(), p.info);
                let list = self.previewable();
                if !list.is_empty() {
                    let at = list.iter().position(|x| *x == cur).map_or(0, |i| i as i64);
                    let next = list[(at + i64::from(by)).rem_euclid(list.len() as i64) as usize].clone();
                    self.preview(ctx, &next);
                    if let Some(p) = self.file_preview.as_mut() {
                        p.info = info;
                    }
                }
            }
            Some(filepreview::Act::Download(id, name)) => self.download(ctx, &id, &name),
            Some(filepreview::Act::NewTab(id)) => {
                #[cfg(target_arch = "wasm32")]
                if let Some(w) = web_sys::window() {
                    let _ = w.open_with_url_and_target(&Self::preview_url(&id), "_blank");
                }
                #[cfg(not(target_arch = "wasm32"))]
                let _ = id;
            }
            None => {}
        }
        true
    }

    /// Downloads file `id`. In the browser the server sends it as an attachment (streamed, so a
    /// big film doesn't sit in the page's memory); the desktop app saves it with a file dialog.
    fn download(&mut self, ctx: &egui::Context, id: &str, name: &str) {
        // Other files in the browser: the server names the download itself.
        #[cfg(target_arch = "wasm32")]
        if items::opens_as(name) != items::Opens::Editor {
            if let Some(w) = web_sys::window() {
                let _ = w.location().set_href(&format!("api/files/{id}"));
            }
            return;
        }
        let name = name.to_string();
        call(&self.store, &self.inbox, ctx, Method::Get, format!("api/files/{id}"), None, move |b| Reply::Download(name, b));
    }

    /// Downloads what is selected: a single file as itself, anything else (several items, or a
    /// folder) as ONE .zip made by the server (`api/zip`), folders with everything below them.
    pub(crate) fn download_items(&mut self, ctx: &egui::Context, items: &[Item]) {
        if let [Item::File(id)] = items {
            let name = self.label_of(&items[0]);
            return self.download(ctx, id, &name);
        }
        let url = Self::zip_url(items);
        #[cfg(target_arch = "wasm32")]
        {
            // The browser saves it as it arrives; the server names it.
            if let Some(w) = web_sys::window() {
                let _ = w.location().set_href(&url);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let name = match items {
                [Item::Folder(p)] => format!("{}.zip", name_of(p)),
                _ => "Starbase.zip".to_string(),
            };
            call(&self.store, &self.inbox, ctx, Method::Get, url, None, move |b| Reply::Download(name, b));
        }
    }

    /// The address of the zip of `items` (files by id, folders by path, one per line).
    pub(crate) fn zip_url(items: &[Item]) -> String {
        let files: Vec<&str> = items.iter().filter_map(|i| if let Item::File(id) = i { Some(id.as_str()) } else { None }).collect();
        let folders: Vec<&str> = items.iter().filter_map(|i| if let Item::Folder(p) = i { Some(p.as_str()) } else { None }).collect();
        format!("api/zip?files={}&folders={}", enc(&files.join(",")), enc(&folders.join("\n")))
    }

    /// A new spreadsheet or document in `folder` (it opens in its own tab or window).
    fn new_file(&mut self, folder: &str, document: bool) {
        if let Err(e) = self.store.new_file(folder, document) {
            self.error = Some(e);
        }
    }

    /// Goes somewhere, remembering where you were for Back.
    fn navigate(&mut self, v: View) {
        if self.view.as_ref() == Some(&v) {
            return;
        }
        if let Some(old) = self.view.take() {
            self.back.push(old);
        }
        self.forward.clear();
        self.view = Some(v);
        self.selected.clear();
        self.search.clear();
        self.filters = Default::default();
        self.renaming = None;
        self.just_made = None;
    }

    fn go_back(&mut self) {
        if let Some(v) = self.back.pop() {
            if let Some(old) = self.view.take() {
                self.forward.push(old);
            }
            self.view = Some(v);
            self.selected.clear();
        }
    }

    fn go_forward(&mut self) {
        if let Some(v) = self.forward.pop() {
            if let Some(old) = self.view.take() {
                self.back.push(old);
            }
            self.view = Some(v);
            self.selected.clear();
        }
    }

    /// The folder above this one, if there is one to go up to.
    fn up_view(&self) -> Option<View> {
        match self.view.clone()? {
            View::Mine(p) if p != self.space => Some(View::Mine(parent_of(&p).to_string())),
            View::Shared(Some(p)) if self.shared_folders.contains(&p) => Some(View::Shared(None)),
            View::Shared(Some(p)) => Some(View::Shared(Some(parent_of(&p).to_string()))),
            View::Everyone(p) if !p.is_empty() => Some(View::Everyone(parent_of(&p).to_string())),
            _ => None,
        }
    }

    fn go_up(&mut self) {
        if let Some(v) = self.up_view() {
            self.navigate(v);
        }
    }

    /// Shows a folder in the right place: mine, shared with me, or (the server's owner) anyone's.
    fn go_folder(&mut self, path: &str) {
        let v = if !self.accounts || under(path, &self.space) {
            View::Mine(path.to_string())
        } else if self.shared_folders.iter().any(|top| under(path, top)) {
            View::Shared(Some(path.to_string()))
        } else if self.admin {
            View::Everyone(path.to_string())
        } else {
            View::Shared(None)
        };
        self.navigate(v);
    }

    /// The folder shown as photos, when "View as photos" is on and a folder is shown (not
    /// Recent, Starred, the bin, or search results).
    fn photo_folder(&self) -> Option<String> {
        if !self.photos.on || !self.store.has_photos() || !self.search.trim().is_empty() {
            return None;
        }
        match self.view.as_ref()? {
            View::Mine(p) | View::Shared(Some(p)) => Some(p.clone()),
            View::Everyone(p) if !p.is_empty() => Some(p.clone()),
            _ => None,
        }
    }

    /// The folder new things go into from here (None: nowhere here, e.g. Recent).
    fn here(&self) -> Option<String> {
        match self.view.as_ref()? {
            View::Mine(p) => Some(p.clone()),
            View::Shared(Some(p)) => self.folder(p).filter(|f| may_edit(&f.role)).map(|_| p.clone()),
            View::Everyone(p) if !p.is_empty() => Some(p.clone()),
            _ => None,
        }
    }

    // ------------------------------------------------------------ what's shown

    /// The folders and files of the current place (or the search results).
    fn contents(&self) -> (Vec<FolderRow>, Vec<FileRow>) {
        let q = self.search.trim().to_lowercase();
        if !q.is_empty() {
            let fl = &self.filters;
            let folders = self
                .folders
                .iter()
                .filter(|f| name_of(&f.path).to_lowercase().contains(&q) && f.path != self.space && f.path.contains('/'))
                .filter(|f| !fl.any() || fl.folder_ok(f, &self.space, self.folder_stats(&f.path).0))
                .cloned()
                .collect();
            let text = self.found_for.to_lowercase() == q;
            let files = self
                .files
                .iter()
                .filter(|f| (f.name.to_lowercase().contains(&q) || (text && self.found.contains_key(&f.id))) && fl.file_ok(f, &self.space))
                .cloned()
                .collect();
            return self.sorted(folders, files);
        }
        let (folders, files): (Vec<FolderRow>, Vec<FileRow>) = match self.view.clone() {
            Some(View::Mine(p) | View::Shared(Some(p)) | View::Everyone(p)) => (
                self.folders.iter().filter(|f| parent_of(&f.path) == p && !f.path.is_empty() && f.path != p).cloned().collect(),
                self.files.iter().filter(|f| f.folder == p).cloned().collect(),
            ),
            Some(View::Shared(None)) => (
                self.folders.iter().filter(|f| self.shared_folders.contains(&f.path)).cloned().collect(),
                self.files.iter().filter(|f| self.shared_files.contains(&f.id)).cloned().collect(),
            ),
            Some(View::Starred) => {
                (self.folders.iter().filter(|f| f.starred).cloned().collect(), self.files.iter().filter(|f| f.starred).cloned().collect())
            }
            Some(View::Recent) => {
                let mut files: Vec<FileRow> = self.files.iter().filter(|f| f.opened.is_some()).cloned().collect();
                files.sort_by_key(|f| std::cmp::Reverse(f.opened));
                files.truncate(60);
                return (Vec::new(), files);
            }
            Some(View::Bin) | None => (Vec::new(), Vec::new()),
        };
        self.sorted(folders, files)
    }

    /// Newest change and total size of everything in a folder (for sorting folders by date/size).
    fn folder_stats(&self, path: &str) -> (i64, u64) {
        let inside = self.files.iter().filter(|f| under(&f.folder, path));
        inside.fold((0, 0), |(t, s), f| (t.max(f.modified), s + f.size))
    }

    /// What an item sorts by (name, date or size), as a comparable key.
    fn sort_key(&self, item: &Item) -> (i64, u64, String) {
        let (name, date, size) = match item {
            Item::Folder(p) => {
                let (d, s) = self.folder_stats(p);
                (self.folder_label(p), d, s)
            }
            Item::File(id) => self.file(id).map_or((String::new(), 0, 0), |f| (f.name.clone(), f.modified, f.size)),
        };
        match self.sort_by {
            1 => (date, 0, name.to_lowercase()),
            2 => (0, size, name.to_lowercase()),
            _ => (0, 0, name.to_lowercase()),
        }
    }

    fn sort_items(&self, items: &mut [Item]) {
        items.sort_by_cached_key(|i| self.sort_key(i));
        if self.descending {
            items.reverse();
        }
        // A folder just made stays first until it is named (to-do 3 row 15).
        if let Some(p) = &self.just_made
            && let Some(k) = items.iter().position(|i| matches!(i, Item::Folder(f) if f == p))
        {
            items[..=k].rotate_right(1);
        }
    }

    fn sorted(&self, folders: Vec<FolderRow>, files: Vec<FileRow>) -> (Vec<FolderRow>, Vec<FileRow>) {
        let mut fo: Vec<Item> = folders.iter().map(|f| Item::Folder(f.path.clone())).collect();
        let mut fi: Vec<Item> = files.iter().map(|f| Item::File(f.id.clone())).collect();
        self.sort_items(&mut fo);
        self.sort_items(&mut fi);
        let fo = fo.iter().filter_map(|i| if let Item::Folder(p) = i { folders.iter().find(|f| f.path == *p).cloned() } else { None }).collect();
        let fi = fi.iter().filter_map(|i| if let Item::File(id) = i { files.iter().find(|f| f.id == *id).cloned() } else { None }).collect();
        (fo, fi)
    }

    /// Everything shown, in the order shown: folders first (the default) or mixed in by the sort.
    fn shown(&self, folders: &[FolderRow], files: &[FileRow]) -> Vec<Item> {
        let mut items: Vec<Item> =
            folders.iter().map(|f| Item::Folder(f.path.clone())).chain(files.iter().map(|f| Item::File(f.id.clone()))).collect();
        if self.mixed && self.view != Some(View::Recent) {
            self.sort_items(&mut items);
        }
        items
    }

    fn save_sort(&self) {
        self.store.set_setting("starsheets_home_sort", &format!("{}{}{}", self.sort_by, u8::from(self.descending), u8::from(self.mixed)));
    }

    /// The Sort menu: Sort by, Sort direction, Folders at top.
    fn sort_menu(&mut self, ui: &mut Ui) {
        let label = RichText::new("Sort").size(house::BODY).color(theme::TEXT);
        let b = match crate::icons::icon("sort") {
            Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(20.0, 20.0)), label),
            None => egui::Button::new(label),
        };
        let r = ui.add(b.fill(Color32::WHITE).stroke(Stroke::new(1.0, theme::BOX)).corner_radius(16).min_size(vec2(88.0, 32.0)));
        egui::Popup::menu(&r).show(|ui| {
            house::menu(ui, 240.0);
            let tick = |on: bool| Some(if on { "check" } else { "" });
            house::menu_inset(ui, |ui| house::caps(ui, "Sort by"));
            for (i, l) in ["Name", "Date modified", "Size"].iter().enumerate() {
                if house::menu_row(ui, tick(self.sort_by == i as u8), l, "", true) {
                    // Dates and sizes read best newest / largest first.
                    if self.sort_by != i as u8 {
                        self.descending = i != 0;
                    }
                    self.sort_by = i as u8;
                    self.save_sort();
                }
            }
            house::menu_sep(ui);
            house::menu_inset(ui, |ui| house::caps(ui, "Sort direction"));
            let (up, down) = match self.sort_by {
                1 => ("Oldest first", "Newest first"),
                2 => ("Smallest first", "Largest first"),
                _ => ("A–Z", "Z–A"),
            };
            if house::menu_row(ui, tick(!self.descending), up, "", true) {
                self.descending = false;
                self.save_sort();
            }
            if house::menu_row(ui, tick(self.descending), down, "", true) {
                self.descending = true;
                self.save_sort();
            }
            house::menu_sep(ui);
            let box_icon = if self.mixed { "check_box_outline_blank" } else { "check_box" };
            if house::menu_row(ui, Some(box_icon), "Folders at top", "", true) {
                self.mixed = !self.mixed;
                self.save_sort();
            }
        });
    }

    fn title(&self) -> Vec<(String, Option<View>)> {
        type Parts = Vec<(String, Option<View>)>;
        let mut parts: Parts = Vec::new();
        fn walk(parts: &mut Parts, top: &str, label: String, path: &str, make: &dyn Fn(String) -> View) {
            parts.push((label, Some(make(top.to_string()))));
            let mut acc = top.to_string();
            for p in path.strip_prefix(top).unwrap_or("").split('/').filter(|p| !p.is_empty()) {
                acc = format!("{acc}/{p}");
                parts.push((p.to_string(), Some(make(acc.clone()))));
            }
        }
        match self.view.clone() {
            Some(View::Mine(p)) => {
                walk(&mut parts, &self.space.clone(), if self.accounts { "My files".into() } else { "All files".into() }, &p, &View::Mine)
            }
            Some(View::Shared(None)) => parts.push(("Shared with me".into(), Some(View::Shared(None)))),
            Some(View::Shared(Some(p))) => {
                parts.push(("Shared with me".into(), Some(View::Shared(None))));
                if let Some(top) = self.shared_folders.iter().find(|t| under(&p, t)).cloned() {
                    walk(&mut parts, &top, name_of(&top).to_string(), &p, &|x| View::Shared(Some(x)));
                }
            }
            Some(View::Everyone(p)) => {
                parts.push(("Everyone's files".into(), Some(View::Everyone(String::new()))));
                if !p.is_empty() {
                    let top = space_of(&p).to_string();
                    let label = self.person(&top);
                    walk(&mut parts, &top, if label == "me" { "My files".into() } else { label }, &p, &View::Everyone);
                }
            }
            Some(View::Recent) => parts.push(("Recent".into(), None)),
            Some(View::Starred) => parts.push(("Starred".into(), None)),
            Some(View::Bin) => parts.push(("Bin".into(), None)),
            None => {}
        }
        parts
    }

    // ------------------------------------------------------------ each frame

    pub fn logic(&mut self, ctx: &egui::Context) {
        if !self.booted {
            self.booted = true;
            self.list = self.store.setting("starsheets_home_list") == Some("1".into());
            if let Some(s) = self.store.setting("starsheets_home_sort").filter(|s| s.len() == 3) {
                let b = s.as_bytes();
                self.sort_by = (b[0] - b'0').min(2);
                self.descending = b[1] == b'1';
                self.mixed = b[2] == b'1';
            }
            self.open_dir = self.store.start_dir();
            if self.store.start_templates() {
                let folder = self.open_dir.clone().unwrap_or_else(|| self.space.clone());
                templates::open(ctx, &folder);
            }
            self.photos.boot(&self.store);
            self.refresh(ctx);
        }
        self.store.poll();
        // The desktop: the list again every little while (a save in a spreadsheet window, a
        // conflicted copy, someone else's change on the server).
        #[cfg(not(target_arch = "wasm32"))]
        if !self.loading && gridcraft_ui_egui::monotonic_ms() - self.listed_at > RELIST_MS {
            self.refresh(ctx);
        }
        #[cfg(not(target_arch = "wasm32"))]
        ctx.request_repaint_after(std::time::Duration::from_millis(RELIST_MS as u64));
        let folder = self.photo_folder();
        self.photos.logic(ctx, &self.store, folder);
        self.ask_text_matches(ctx);
        let replies: Vec<Reply> = std::mem::take(&mut *self.inbox.borrow_mut());
        for r in replies {
            match r {
                Reply::Listed(v) => self.take_list(v),
                Reply::Bin(b) => {
                    self.bin = b;
                    self.loading = false;
                }
                Reply::Refresh => self.refresh(ctx),
                Reply::NewFolder(path) => {
                    self.refresh(ctx);
                    self.undo.made(path.clone());
                    self.selected = vec![Item::Folder(path.clone())];
                    self.renaming = Some((Item::Folder(path.clone()), name_of(&path).to_string(), true));
                    self.just_made = Some(path);
                }
                Reply::Thumb(id, version, img) => {
                    self.fetching = self.fetching.saturating_sub(1);
                    let t = match img {
                        Some(img) => Thumb::Ready(ctx.load_texture(format!("thumb-{id}"), img, egui::TextureOptions::LINEAR)),
                        None => Thumb::None,
                    };
                    self.thumbs.insert(id, (version, t));
                }
                Reply::Access(item, v) => {
                    if let Some((i, a)) = self.details.as_mut()
                        && *i == item
                    {
                        *a = Some(v);
                    }
                }
                Reply::Did(group, r) => {
                    let now = ctx.input(|i| i.time);
                    let step = match r {
                        Ok(step) => step,
                        Err(e) => {
                            if !e.is_empty() {
                                self.error = Some(e);
                            }
                            None
                        }
                    };
                    self.undo.answered(group, step, now);
                    if !self.undo.waiting.contains_key(&group) {
                        self.refresh(ctx);
                    }
                }
                Reply::Undone(r) => self.undone(ctx, r),
                Reply::Download(name, bytes) => {
                    if let Err(e) = self.store.download(&name, &bytes) {
                        self.error = Some(e);
                    }
                }
                Reply::Found(q, hits) => {
                    if q == self.search.trim() {
                        self.found = hits.into_iter().collect();
                        self.found_for = q;
                    }
                }
                Reply::Error(e) => {
                    self.loading = false;
                    // An empty error is "please sign in" (the sign-in box or a renewal follows):
                    // asking again would only get 401 again, over and over.
                    if !e.is_empty() {
                        self.error = Some(e);
                        // The desktop lists again by itself a little later (a server that can't
                        // be reached isn't asked again and again).
                        #[cfg(target_arch = "wasm32")]
                        self.refresh(ctx);
                    }
                }
            }
        }
    }

    /// Files dropped on the page or picked with Upload: saved into the folder shown, any type,
    /// under their own name (`as_is`: photo.jpg stays photo.jpg).
    pub fn upload(&mut self, ctx: &egui::Context, name: String, bytes: Vec<u8>) {
        let folder = self.here().unwrap_or_else(|| self.space.clone());
        call(
            &self.store,
            &self.inbox,
            ctx,
            Method::Post,
            format!("api/files?name={}&folder={}&as_is=1", enc(&name), enc(&folder)),
            Some(bytes),
            refresh_after,
        );
    }

    fn want_thumb(&mut self, ctx: &egui::Context, f: &FileRow) {
        // Spreadsheets, documents, photos and PDFs have a picture: other files show their type's
        // icon (so does any file the server has no picture of).
        if items::opens_as(&f.name) != items::Opens::Editor && !matches!(items::file_kind(&f.name), "image" | "pdf") {
            return;
        }
        if self.thumbs.get(&f.id).is_some_and(|(v, _)| *v == f.version) || self.fetching >= THUMBS_AT_ONCE {
            return;
        }
        // Keep showing the old picture while the new one comes.
        if let Some((v, _)) = self.thumbs.get_mut(&f.id) {
            *v = f.version.clone();
        } else {
            self.thumbs.insert(f.id.clone(), (f.version.clone(), Thumb::Loading));
        }
        self.fetching += 1;
        let (id, version) = (f.id.clone(), f.version.clone());
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        let url = format!("api/files/{id}/thumb");
        self.store.call(
            ctx,
            Method::Get,
            &url,
            None,
            Box::new(move |answer| {
                let img = match answer {
                    Ok((200, bytes)) => decode_png(&bytes),
                    _ => None,
                };
                inbox.borrow_mut().push(Reply::Thumb(id, version, img));
                ctx2.request_repaint();
            }),
        );
    }

    // ------------------------------------------------------------ drawing

    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.templates_ui(&ctx);
        ui.painter().rect_filled(ui.max_rect(), 0, theme::PAGE);
        egui::Panel::top("home_top")
            .show_separator_line(false)
            .exact_size(TOP)
            .frame(egui::Frame::NONE.fill(theme::PAGE).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ui, |ui| self.top_ui(ui));
        egui::Panel::left("home_left")
            .show_separator_line(false)
            .exact_size(LEFT)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(theme::PAGE).inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 8 }))
            .show(ui, |ui| self.left_ui(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(Color32::WHITE)
                    .corner_radius(egui::CornerRadius { nw: 16, ..Default::default() })
                    .inner_margin(egui::Margin { left: 24, right: 16, top: 16, bottom: 0 }),
            )
            .show(ui, |ui| self.main_ui(ui));
        self.drag_ui(&ctx);
        self.move_to_ui(&ctx);
        self.details_ui(&ctx);
        self.pending_move_ui(&ctx);
        self.note_ui(&ctx);
        // The photo viewer covers everything and takes the keys while it's open.
        let dialog = self.photos.pick_busy();
        if !viewer::ui(&mut self.photos, &ctx, &self.store) && !dialog && !self.file_preview_ui(&ctx) {
            self.keys(&ctx);
        }
        // Photos' Add to album, Add photos and its note: over the grid and the viewer.
        self.photos.overlays(&ctx, &self.store);
        // Connected to a server, or disconnected: the Drive starts again over the new files.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(store) = self.connect.ui(&ctx) {
            *self = Home { store, connect: std::mem::take(&mut self.connect), ..Default::default() };
            ctx.request_repaint();
        }
    }

    fn top_ui(&mut self, ui: &mut Ui) {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if let Some(img) = crate::icons::drive_logo(36.0) {
                ui.add(img);
            }
            ui.add_space(house::S);
            ui.label(RichText::new("Starbase Drive").size(22.0).color(theme::TEXT));
            // The search box starts where it always has, whatever the name's width.
            let x = ui.cursor().min.x - ui.max_rect().left();
            ui.add_space((LEFT - 28.0 - x - ui.spacing().item_spacing.x).max(house::M));
            let search = house::field(&mut self.search, house::SEARCH_TEXT)
                .id(egui::Id::unique("home_search"))
                .hint_text(RichText::new("Search in Drive").size(house::SEARCH_TEXT))
                .frame(egui::Frame::NONE)
                .margin(vec2(0.0, 0.0))
                .min_size(vec2(0.0, 24.0))
                .desired_width((ui.available_width() - 140.0).clamp(200.0, 700.0));
            let w = (ui.available_width() - 100.0).clamp(220.0, 740.0);
            ui.allocate_ui_with_layout(vec2(w, 48.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // One rounded box: the field itself draws no frame of its own.
                egui::Frame::NONE.fill(theme::TOOLBAR).corner_radius(24).inner_margin(egui::Margin::symmetric(20, 12)).show(ui, |ui| {
                    if let Some(img) = crate::icons::icon("search") {
                        ui.add(img.fit_to_exact_size(vec2(20.0, 20.0)));
                    }
                    ui.add_space(house::S);
                    ui.add(search);
                });
            });
            // Who is signed in (the server's accounts; none in the desktop app's local mode).
            #[cfg(target_arch = "wasm32")]
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                crate::account::me_ui(ui);
            });
            // The desktop app: "Connect to a server…", or who is signed in on the server.
            #[cfg(not(target_arch = "wasm32"))]
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.connect.menu_ui(ui, &self.store);
            });
        });
    }
}

/// A folder's icon: filled in its colour if it has one, else the grey outline (with people on it
/// when it's shared).
fn folder_icon(ui: &Ui, f: &FolderRow, rect: Rect) {
    let img = match f.colour {
        Some(c) => crate::icons::icon("folder_fill").map(|i| i.tint(c)),
        None => crate::icons::icon(if f.shared { "folder_shared" } else { "folder" }),
    };
    if let Some(img) = img {
        img.paint_at(ui, rect);
    }
}

impl Home {
    /// How a folder is called on screen: someone's own folder by their name.
    fn folder_label(&self, path: &str) -> String {
        if self.accounts && !path.contains('/') {
            let p = self.person(path);
            return if p == "me" { "My files".into() } else { p };
        }
        name_of(path).to_string()
    }
}

fn size_text(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{:.0} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// Whether a role from the server may change a file or add to a folder: "comment" (documents
/// only) and "view" may not.
pub(crate) fn may_edit(role: &str) -> bool {
    !matches!(role, "view" | "comment")
}
