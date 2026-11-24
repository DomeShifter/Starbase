//! The Share dialog (live editing step 4), as in Google Sheets: who has access to this file
//! (owner, can edit, can view), adding people with an account on this server, and general
//! access: restricted, or anyone with the link (signed in here) can view or edit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{RichText, vec2};
use serde_json::{Value as Json, json};

use crate::account::{Me, avatar};
use crate::files::{Method, fetch};
use crate::house;
use crate::theme;

/// Asks for the Share dialog (the Share button sets it in egui memory).
pub const REQUEST: &str = "starsheets_share_request";
/// Asks for the Share dialog for something on the home screen: (sharing url, page link, name).
const ITEM_REQUEST: &str = "starsheets_share_item";

/// Opens the Share dialog for a file (by id) or a folder (by path) from the home screen.
pub fn share_item(ctx: &egui::Context, folder: bool, key: &str, name: &str) {
    let (url, link) = if folder {
        (format!("api/folders/sharing?path={}", crate::files::enc(key)), format!("?dir={}", crate::files::enc(key)))
    } else {
        (format!("api/files/{key}/sharing"), format!("?file={key}"))
    };
    ctx.data_mut(|m| m.insert_temp(egui::Id::unique(ITEM_REQUEST), (url, link, name.to_string())));
}

thread_local! {
    /// Whether the open file can be shared (signed in, and it's on the server).
    static SHAREABLE: Cell<bool> = const { Cell::new(false) };
}

pub fn can_share() -> bool {
    SHAREABLE.with(Cell::get)
}

#[derive(Clone)]
struct Person {
    username: String,
    name: String,
    colour: String,
    role: String,
}

enum Reply {
    Loaded { people: Vec<Person>, link: String, you: String },
    Users(Vec<Me>),
    Saved,
    Error(String),
}

const ROLES: [&str; 3] = ["Viewer", "Editor", "Remove access"];
const LINKS: [&str; 3] = ["Restricted: only people added can open it", "Anyone with the link can view", "Anyone with the link can edit"];

fn role_index(r: &str) -> usize {
    if r == "edit" { 1 } else { 0 }
}

#[derive(Default)]
pub struct Share {
    inbox: Rc<RefCell<Vec<Reply>>>,
    open: bool,
    /// Where its sharing is read and saved, and the link people are given ("?file=…").
    url: String,
    page: String,
    name: String,
    loaded: bool,
    people: Vec<Person>,
    /// Role picks per person (index in ROLES), as shown.
    picks: Vec<usize>,
    link: usize,
    you: String,
    users: Vec<Me>,
    add: usize,
    add_role: usize,
    error: Option<String>,
    copied: bool,
    busy: bool,
    /// Share was pressed on a spreadsheet not saved yet: it is being saved, and the dialog opens
    /// once it is on the server.
    pending: bool,
}

fn call(
    inbox: &Rc<RefCell<Vec<Reply>>>,
    ctx: &egui::Context,
    method: Method,
    url: String,
    body: Option<Json>,
    then: impl FnOnce(Json) -> Reply + 'static,
) {
    let (inbox, ctx) = (inbox.clone(), ctx.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let reply = match fetch(method, &url, body.map(|b| serde_json::to_vec(&b).unwrap_or_default()), None).await {
            Ok((s, _, bytes)) => {
                let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
                if s < 400 { then(v) } else { Reply::Error(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string)) }
            }
            Err(e) => Reply::Error(e),
        };
        inbox.borrow_mut().push(reply);
        ctx.request_repaint();
    });
}

fn loaded(v: Json) -> Reply {
    let people = v["people"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| Person {
            username: p["username"].as_str().unwrap_or("").into(),
            name: p["name"].as_str().unwrap_or("").into(),
            colour: p["colour"].as_str().unwrap_or("#5F6368").into(),
            role: p["role"].as_str().unwrap_or("view").into(),
        })
        .collect();
    Reply::Loaded { people, link: v["link"].as_str().unwrap_or("none").into(), you: v["you"].as_str().unwrap_or("view").into() }
}

impl Share {
    pub fn logic(&mut self, app: &gridcraft_ui_egui::SheetApp, ctx: &egui::Context, files: &crate::files::Files) {
        let file = files.linked().filter(|(doc, _)| app.session.active().is_some_and(|d| d.uid == *doc));
        // Signed in is enough: a spreadsheet not saved yet (even an empty one) is saved first.
        let signed_in = crate::account::me().is_some();
        SHAREABLE.with(|s| s.set(signed_in && app.session.active().is_some()));
        let item = ctx.data_mut(|m| m.remove_temp::<(String, String, String)>(egui::Id::unique(ITEM_REQUEST)));
        let mut asked = ctx.data_mut(|m| m.remove_temp::<()>(egui::Id::unique(REQUEST))).is_some();
        if asked && file.is_none() && signed_in {
            self.pending = true;
            ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::files::SAVE_NOW), ()));
        }
        if self.pending && file.is_some() {
            self.pending = false;
            asked = true;
        }
        if !signed_in {
            self.pending = false;
        }
        let target = match (item, file) {
            (Some(t), _) => Some(t),
            (None, Some((_, id))) if asked => {
                Some((format!("api/files/{id}/sharing"), format!("?file={id}"), app.session.active().map(|d| d.title.clone()).unwrap_or_default()))
            }
            _ => None,
        };
        if let Some((url, page, name)) = target {
            *self = Share { inbox: self.inbox.clone(), open: true, url: url.clone(), page, name, ..Share::default() };
            call(&self.inbox, ctx, Method::Get, url, None, loaded);
            call(&self.inbox, ctx, Method::Get, "api/users".into(), None, |v| {
                Reply::Users(
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .map(|u| Me {
                            username: u["username"].as_str().unwrap_or("").into(),
                            name: u["name"].as_str().unwrap_or("").into(),
                            colour: u["colour"].as_str().unwrap_or("").into(),
                            owner: false,
                        })
                        .collect(),
                )
            });
        }
        let replies: Vec<Reply> = std::mem::take(&mut *self.inbox.borrow_mut());
        for r in replies {
            self.busy = false;
            match r {
                Reply::Loaded { people, link, you } => {
                    self.picks = people.iter().map(|p| role_index(&p.role)).collect();
                    self.people = people;
                    self.link = match link.as_str() {
                        "view" => 1,
                        "edit" => 2,
                        _ => 0,
                    };
                    self.you = you;
                    self.loaded = true;
                }
                Reply::Users(u) => self.users = u,
                Reply::Saved => self.open = false,
                Reply::Error(e) => self.error = Some(e),
            }
        }
    }

    fn link_url(&self) -> String {
        let base = web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default();
        let base = base.split(['?', '#']).next().unwrap_or("").to_string();
        format!("{base}{}", self.page)
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut close = false;
        let mut cancel = false;
        let mut save = false;
        let owner = self.you == "owner";
        let title = format!("Share \"{}\"", self.name);
        house::dialog(ctx, "starsheets_share", 520.0, &title, Some(&mut close), |ui| {
            if !self.loaded {
                house::note(ui, "Loading…");
                return;
            }
            // Add people: everyone with an account here who isn't on the list yet.
            let others: Vec<Me> = self.users.iter().filter(|u| !self.people.iter().any(|p| p.username == u.username)).cloned().collect();
            if owner && !others.is_empty() {
                house::label(ui, "Add people", "");
                ui.horizontal(|ui| {
                    let labels: Vec<String> = others.iter().map(|u| format!("{} ({})", u.name, u.username)).collect();
                    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                    self.add = self.add.min(refs.len().saturating_sub(1));
                    house::select(ui, "starsheets_share_add", 250.0, &mut self.add, &refs, &[]);
                    house::select(ui, "starsheets_share_add_role", 110.0, &mut self.add_role, &ROLES[..2], &[]);
                    if ui.add(house::secondary("Add")).clicked()
                        && let Some(u) = others.get(self.add)
                    {
                        self.people.push(Person {
                            username: u.username.clone(),
                            name: u.name.clone(),
                            colour: u.colour.clone(),
                            role: if self.add_role == 1 { "edit".into() } else { "view".into() },
                        });
                        self.picks.push(self.add_role);
                    }
                });
                house::gap(ui);
            }
            house::label(ui, "People with access", "");
            for (i, p) in self.people.iter().enumerate() {
                ui.horizontal(|ui| {
                    avatar(ui, &p.name, &p.colour, 30.0);
                    ui.add_space(house::S);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&p.name).size(house::BODY).color(theme::TEXT));
                        house::note(ui, &p.username);
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if p.role == "owner" {
                            house::note(ui, "Owner");
                        } else if owner {
                            house::select(ui, &format!("starsheets_share_role_{i}"), 150.0, &mut self.picks[i], &ROLES, &[1]);
                        } else {
                            house::note(ui, if p.role == "edit" { "Editor" } else { "Viewer" });
                        }
                    });
                });
                ui.add_space(house::XS);
            }
            house::gap(ui);
            house::label(ui, "General access", "\"Anyone\" means anyone with an account on this server.");
            if owner {
                house::select(ui, "starsheets_share_link", 400.0, &mut self.link, &LINKS, &[0]);
            } else {
                house::text(ui, LINKS[self.link]);
                house::note(ui, "Only the owner can change who has access.");
            }
            house::gap(ui);
            ui.horizontal(|ui| {
                let mut url = self.link_url();
                ui.add(egui::TextEdit::singleline(&mut url).desired_width(360.0).margin(vec2(house::S, house::XS + 2.0)));
                if ui.add(house::secondary(if self.copied { "Copied" } else { "Copy link" })).clicked() {
                    if let Some(w) = web_sys::window() {
                        let _ = w.navigator().clipboard().write_text(&url);
                    }
                    self.copied = true;
                }
            });
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
            }
            let labels: &[&str] = if owner { &["Cancel", "Done"] } else { &["Done"] };
            match house::actions(ui, labels) {
                Some(i) if owner && i == 1 => save = true,
                Some(_) => cancel = true,
                None => {}
            }
        });
        if close || cancel {
            self.open = false;
        } else if save && !self.busy {
            self.busy = true;
            let mut people = serde_json::Map::new();
            for (p, pick) in self.people.iter().zip(&self.picks) {
                if p.role != "owner" {
                    people.insert(p.username.clone(), json!(["view", "edit", "remove"][(*pick).min(2)]));
                }
            }
            let link = ["none", "view", "edit"][self.link.min(2)];
            let body = json!({ "people": people, "link": link });
            call(&self.inbox, ctx, Method::Post, self.url.clone(), Some(body), |_| Reply::Saved);
        }
    }
}
