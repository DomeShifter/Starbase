//! The Share dialog for a document, as in Google Docs: who has access (owner, can edit, can
//! comment, can view), adding people with an account on this server, and general access
//! (restricted, or anyone with the link, signed in here, can view, comment or edit). A commenter
//! may read the document and add, reply to and resolve comments, not change its text (the server
//! checks their saves: starsheets/crates/server/src/commenting.rs).
//!
//! Same server API and same choices as StarSheets' Share dialog (StarSheets `app/src/share.rs`),
//! so a document shared from the home screen and one shared from here behave the same:
//! `GET/POST api/files/<id>/sharing` and `GET api/users`. Kept as its own copy, drawn with
//! StarDocs' colours, until the two apps share one repo and one house-style crate.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Align, Color32, Layout, RichText, Ui, vec2};
use serde_json::{Value as Json, json};
use wordcraft_ui_egui::WordApp;

use crate::server::{Link, api, fetch};
use crate::shell::{BLUE, SHARE, TEXT, TEXT_DIM};

const ERROR: Color32 = Color32::from_rgb(0xB3, 0x26, 0x1E);

#[derive(Clone)]
struct Person {
    username: String,
    name: String,
    colour: String,
    role: String,
}

enum Reply {
    Loaded { people: Vec<Person>, link: String, you: String },
    Users(Vec<Person>),
    Saved,
    Error(String),
}

const ROLES: [&str; 4] = ["Viewer", "Commenter", "Editor", "Remove access"];
/// What each of ROLES is called on the server.
const ROLE_NAMES: [&str; 4] = ["view", "comment", "edit", "remove"];
const LINKS: [&str; 4] = [
    "Restricted: only people added can open it",
    "Anyone with the link can view",
    "Anyone with the link can comment",
    "Anyone with the link can edit",
];
const LINK_NAMES: [&str; 4] = ["none", "view", "comment", "edit"];

/// A role's index in ROLES (an unknown one shows as Viewer).
fn role_index(role: &str) -> usize {
    ROLE_NAMES.iter().position(|n| *n == role).unwrap_or(0)
}

#[derive(Default)]
pub struct Share {
    inbox: Rc<RefCell<Vec<Reply>>>,
    open: bool,
    /// Share was pressed on a document not saved yet: it is being saved, and the dialog opens once
    /// the server has given it an id.
    pending: bool,
    id: String,
    name: String,
    loaded: bool,
    people: Vec<Person>,
    /// Role picks per person (index in ROLES), as shown.
    picks: Vec<usize>,
    link: usize,
    you: String,
    users: Vec<Person>,
    add: usize,
    add_role: usize,
    error: Option<String>,
    copied: bool,
    copy_failed: bool,
    busy: bool,
}

fn call(
    inbox: &Rc<RefCell<Vec<Reply>>>,
    ctx: &egui::Context,
    method: &'static str,
    url: String,
    body: Option<Json>,
    then: impl FnOnce(Json) -> Reply + 'static,
) {
    let (inbox, ctx) = (inbox.clone(), ctx.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let bytes = body.map(|b| serde_json::to_vec(&b).unwrap_or_default());
        let reply = match fetch(method, &url, None, bytes.as_deref()).await {
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

fn person(p: &Json) -> Person {
    Person {
        username: p["username"].as_str().unwrap_or("").into(),
        name: p["name"].as_str().unwrap_or("").into(),
        colour: p["colour"].as_str().unwrap_or("#5F6368").into(),
        role: p["role"].as_str().unwrap_or("view").into(),
    }
}

fn parse_colour(c: &str) -> Color32 {
    Color32::from_hex(c).unwrap_or(Color32::from_rgb(0x5F, 0x63, 0x68))
}

fn avatar(ui: &mut Ui, name: &str, colour: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.center(), size / 2.0, parse_colour(colour));
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    p.text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(size * 0.5), Color32::WHITE);
}

fn pick(ui: &mut Ui, id: &str, width: f32, current: &mut usize, labels: &[&str]) {
    egui::ComboBox::from_id_salt(id).width(width).selected_text(RichText::new(labels[(*current).min(labels.len() - 1)]).color(TEXT)).show_ui(
        ui,
        |ui| {
            for (i, l) in labels.iter().enumerate() {
                ui.selectable_value(current, i, *l);
            }
        },
    );
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(text).size(14.0).strong().color(TEXT));
    ui.add_space(4.0);
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(TEXT_DIM));
}

fn button(text: &str, fill: Color32, colour: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(14.0).color(colour)).fill(fill).corner_radius(18.0).min_size(vec2(84.0, 36.0))
}

/// Copies `text`. Browsers offer the clipboard API only on https (or localhost), so on a plain
/// http address this falls back to the old way: a hidden text box, selected, and "copy".
fn copy_text(text: &str) -> bool {
    use wasm_bindgen::JsCast as _;
    let Some(w) = web_sys::window() else { return false };
    let nav = w.navigator();
    let has_api = js_sys::Reflect::get(&nav, &"clipboard".into()).is_ok_and(|c| !c.is_undefined() && !c.is_null());
    if has_api {
        let _ = nav.clipboard().write_text(text);
        return true;
    }
    let Some(doc) = w.document() else { return false };
    let Ok(area) = doc.create_element("textarea") else { return false };
    let Ok(area) = area.dyn_into::<web_sys::HtmlTextAreaElement>() else { return false };
    area.set_value(text);
    let _ = area.set_attribute("style", "position:fixed;top:0;left:0;opacity:0");
    let Some(body) = doc.body() else { return false };
    if body.append_child(&area).is_err() {
        return false;
    }
    area.select();
    let done = doc.dyn_ref::<web_sys::HtmlDocument>().is_some_and(|d| d.exec_command("copy").unwrap_or(false));
    area.remove();
    done
}

impl Share {
    /// The Share button: opens the dialog, saving the document first if the server hasn't got it yet.
    pub fn ask(&mut self, app: &mut WordApp, link: &Link, ctx: &egui::Context) {
        if link.id().is_none() {
            self.pending = true;
            link.save_now(app);
        } else {
            self.start(link, ctx);
        }
    }

    fn start(&mut self, link: &Link, ctx: &egui::Context) {
        let Some(id) = link.id() else { return };
        let name = Some(link.name()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled document".to_string());
        let name = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s).to_string();
        *self = Share { inbox: self.inbox.clone(), open: true, id: id.clone(), name, ..Share::default() };
        call(&self.inbox, ctx, "GET", api(&format!("api/files/{id}/sharing")), None, |v| Reply::Loaded {
            people: v["people"].as_array().into_iter().flatten().map(person).collect(),
            link: v["link"].as_str().unwrap_or("none").into(),
            you: v["you"].as_str().unwrap_or("view").into(),
        });
        call(&self.inbox, ctx, "GET", api("api/users"), None, |v| Reply::Users(v.as_array().into_iter().flatten().map(person).collect()));
    }

    /// Every frame: the pending first save, and the server's replies.
    pub fn logic(&mut self, link: Option<&Link>, ctx: &egui::Context) {
        if self.pending
            && let Some(link) = link
            && link.id().is_some()
        {
            self.pending = false;
            self.start(link, ctx);
        }
        let replies: Vec<Reply> = std::mem::take(&mut *self.inbox.borrow_mut());
        for r in replies {
            self.busy = false;
            match r {
                Reply::Loaded { people, link, you } => {
                    self.picks = people.iter().map(|p| role_index(&p.role)).collect();
                    self.people = people;
                    self.link = LINK_NAMES.iter().position(|n| *n == link).unwrap_or(0);
                    self.you = you;
                    self.loaded = true;
                }
                Reply::Users(u) => self.users = u,
                Reply::Saved => self.open = false,
                Reply::Error(e) => self.error = Some(e),
            }
        }
    }

    /// Whether the Share button is waiting for the first save.
    /// The dialog is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn waiting(&self) -> bool {
        self.pending
    }

    fn link_url(&self) -> String {
        let base = web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default();
        let base = base.split(['?', '#']).next().unwrap_or("").to_string();
        format!("{base}?file={}", self.id)
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let owner = self.you == "owner";
        let mut close = false;
        let mut save = false;
        // Anchored by its top edge, not its centre: adding a person makes the dialog taller, and
        // centred it would jump up by half a row each time.
        let id = egui::Id::unique("stardocs_share");
        let top = (ctx.content_rect().height() * 0.12).clamp(24.0, 120.0);
        let modal = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, vec2(0.0, top)))
            .frame(egui::Frame::popup(&ctx.global_style()).fill(Color32::WHITE).corner_radius(8.0).inner_margin(24.0));
        let shown = modal.show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new(format!("Share \"{}\"", self.name)).size(22.0).color(TEXT));
            if !self.loaded {
                ui.add_space(12.0);
                note(ui, "Loading…");
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).size(12.0).color(ERROR));
                }
                ui.add_space(12.0);
                if ui.add(button("Close", Color32::WHITE, BLUE)).clicked() {
                    close = true;
                }
                return;
            }
            // Add people: everyone with an account here who isn't on the list yet.
            let others: Vec<Person> = self.users.iter().filter(|u| !self.people.iter().any(|p| p.username == u.username)).cloned().collect();
            if owner && others.is_empty() {
                // Kept in place (not hidden) so the dialog doesn't jump when the last person is added.
                heading(ui, "Add people");
                note(ui, "Everyone with an account on this server is on the list below.");
            }
            if owner && !others.is_empty() {
                heading(ui, "Add people");
                ui.horizontal(|ui| {
                    let labels: Vec<String> = others.iter().map(|u| format!("{} ({})", u.name, u.username)).collect();
                    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                    self.add = self.add.min(refs.len() - 1);
                    pick(ui, "stardocs_share_add", 250.0, &mut self.add, &refs);
                    pick(ui, "stardocs_share_add_role", 100.0, &mut self.add_role, &ROLES[..3]);
                    let h = ui.spacing().interact_size.y;
                    let add = egui::Button::new(RichText::new("Add").size(14.0).color(BLUE))
                        .fill(Color32::WHITE)
                        .corner_radius(h / 2.0)
                        .min_size(vec2(64.0, h));
                    if ui.add(add).clicked()
                        && let Some(u) = others.get(self.add)
                    {
                        self.people.push(Person { role: ROLE_NAMES[self.add_role.min(2)].into(), ..u.clone() });
                        self.picks.push(self.add_role);
                    }
                });
            }
            heading(ui, "People with access");
            for (i, p) in self.people.iter().enumerate() {
                ui.horizontal(|ui| {
                    avatar(ui, &p.name, &p.colour, 30.0);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&p.name).size(14.0).color(TEXT));
                        note(ui, &p.username);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if p.role == "owner" {
                            note(ui, "Owner");
                        } else if owner {
                            pick(ui, &format!("stardocs_share_role_{i}"), 130.0, &mut self.picks[i], &ROLES);
                        } else {
                            note(ui, ROLES[role_index(&p.role).min(2)]);
                        }
                    });
                });
                ui.add_space(4.0);
            }
            heading(ui, "General access");
            note(ui, "\"Anyone\" means anyone with an account on this server.");
            ui.add_space(4.0);
            if owner {
                pick(ui, "stardocs_share_link", 380.0, &mut self.link, &LINKS);
            } else {
                ui.label(RichText::new(LINKS[self.link]).size(14.0).color(TEXT));
                note(ui, "Only the owner can change who has access.");
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let mut url = self.link_url();
                let width = ui.available_width() - 110.0;
                // The button takes the text box's height, so the two line up.
                let h = ui.add(egui::TextEdit::singleline(&mut url).desired_width(width).margin(vec2(8.0, 6.0))).rect.height();
                let copy =
                    button(if self.copied { "Copied" } else { "Copy link" }, Color32::WHITE, BLUE).corner_radius(h / 2.0).min_size(vec2(84.0, h));
                if ui.add(copy).clicked() {
                    self.copied = copy_text(&url);
                    self.copy_failed = !self.copied;
                }
            });
            if self.copied {
                note(ui, "Link copied. Anyone you send it to signs in here first.");
            } else if self.copy_failed {
                note(ui, "Couldn't copy here: select the link above and copy it yourself.");
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).size(12.0).color(ERROR));
            }
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if owner {
                    if ui.add(button("Done", BLUE, Color32::WHITE)).clicked() {
                        save = true;
                    }
                    if ui.add(button("Cancel", Color32::WHITE, BLUE)).clicked() {
                        close = true;
                    }
                } else if ui.add(button("Done", SHARE, TEXT)).clicked() {
                    close = true;
                }
            });
        });
        if close || shown.should_close() {
            self.open = false;
        } else if save && !self.busy {
            self.busy = true;
            self.error = None;
            let mut people = serde_json::Map::new();
            for (p, pick) in self.people.iter().zip(&self.picks) {
                if p.role != "owner" {
                    people.insert(p.username.clone(), json!(ROLE_NAMES[(*pick).min(3)]));
                }
            }
            let general = LINK_NAMES[self.link.min(3)];
            let body = json!({ "people": people, "link": general });
            call(&self.inbox, ctx, "POST", api(&format!("api/files/{}/sharing", self.id)), Some(body), |_| Reply::Saved);
        }
    }
}
