//! Accounts (live editing step 2): signing in with a username and password kept on the
//! StarSheets server, the first-time setup of the owner's account, your avatar menu (name and
//! colour, password, people, sign out), the owner's list of people, and sign-up (people making
//! their own account, when the owner allows it).
//! No sign-in through other companies (docs/decisions.md).

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Align2, Color32, RichText, Sense, Ui, vec2};
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast;

use crate::files::{Method, fetch};
use crate::house;
use crate::theme;

/// Colours people can pick (the same ones the server hands out).
const COLOURS: [&str; 8] = ["#D93025", "#188038", "#E37400", "#9334E6", "#007B83", "#E52592", "#B06000", "#5F6368"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Me {
    pub username: String,
    pub name: String,
    pub colour: String,
    pub owner: bool,
}

thread_local! {
    static ME: RefCell<Option<Me>> = const { RefCell::new(None) };
    /// Set when a request came back "please sign in".
    /// The People dialog's link-preview checkbox as last drawn, for the test hook (None until drawn).
    static LINK_PREVIEW_SHOWN: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static SIGN_IN: RefCell<bool> = const { RefCell::new(false) };
    /// Set when a request came back "please sign in" for a single sign-on session that can be
    /// renewed at the provider without asking (files.rs `fetch`).
    static RENEW: RefCell<bool> = const { RefCell::new(false) };
    /// Set when the live connection keeps failing to reconnect: look whether the sign-in ended.
    static CHECK: RefCell<bool> = const { RefCell::new(false) };
    /// A cell is being typed in (web.rs): not a moment to leave the page.
    static TYPING: RefCell<bool> = const { RefCell::new(false) };
    /// The signed-in account has a password (accounts made through single sign-on have none).
    static HAS_PASSWORD: RefCell<bool> = const { RefCell::new(true) };
    /// The page is on its way to the provider to renew the session.
    static LEAVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The server said this tab may see Drive (signed in, or sign-in off). Until then nothing of
    /// Drive or a file is drawn: the sign-in page (index.html) is a page of its own.
    static APP_SHOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether Drive (or an open file) may be drawn: not before the server has said who is signed
/// in, and not while signed out (web.rs draws only the sign-in page and its dialogs then).
pub fn app_shown() -> bool {
    APP_SHOWN.with(std::cell::Cell::get)
}

/// The person signed in in this tab (None with sign-in off).
pub fn me() -> Option<Me> {
    ME.with(|m| m.borrow().clone())
}

/// A request needed sign-in: show the sign-in box.
pub fn ask_sign_in() {
    SIGN_IN.with(|s| *s.borrow_mut() = true);
}

/// Opens one of the avatar menu's dialogs, as the menu does (the test hook: `account:profile`,
/// `account:password`, `account:people`).
pub fn request(ctx: &egui::Context, what: &str) -> Result<(), String> {
    if !matches!(what, "profile" | "password" | "people" | "storage" | "backups" | "devices" | "settings" | "photos-settings") {
        return Err(format!("no account dialog `{what}`"));
    }
    ctx.data_mut(|m| m.insert_temp(egui::Id::unique(REQUEST), what.to_string()));
    Ok(())
}

/// A request found the single sign-on session ended but renewable: renew it at a quiet moment.
pub fn renewal_needed() {
    RENEW.with(|r| *r.borrow_mut() = true);
}

/// The live connection can't get back in: check whether the sign-in has ended.
pub fn check_session() {
    CHECK.with(|c| *c.borrow_mut() = true);
}

/// Whether a cell is being typed in (set every frame by web.rs).
pub fn set_typing(on: bool) {
    TYPING.with(|t| *t.borrow_mut() = on);
}

/// Milliseconds since 1970 (the page's clock).
fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// The session storage key that remembers a renewal was just started (so the page doesn't go
/// round in circles, and a renewal that failed before the session ended isn't shown as an
/// error).
const RENEWING: &str = "starsheets_renewing";

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.session_storage().ok().flatten())
}

/// When a renewal was started from this tab (ms), if one was.
fn renewing_since() -> Option<f64> {
    session_storage().and_then(|s| s.get_item(RENEWING).ok().flatten()).and_then(|v| v.parse().ok())
}

pub fn parse_colour(hex: &str) -> Color32 {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(0x80);
    Color32::from_rgb(p(0), p(2), p(4))
}

/// A round avatar with the first letter of `name`. Returns its response (hover, click).
pub fn avatar(ui: &mut Ui, name: &str, colour: &str, size: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let p = ui.painter();
    p.circle_filled(rect.center(), size / 2.0, parse_colour(colour));
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    p.text(rect.center(), Align2::CENTER_CENTER, letter, egui::FontId::proportional(size * 0.5), Color32::WHITE);
    r
}

/// The account menu's id (egui popup memory), so the test hook can open it and say if it's open.
fn menu_id() -> egui::Id {
    egui::Id::unique("starsheets_account_menu")
}

/// The test hook's `account` part of the snapshot: signed in or not, the sign-in page showing,
/// the account menu open, and where signing in goes on to.
pub fn test_state(ctx: &egui::Context) -> Json {
    let page = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("ss_signin_back"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        .is_some_and(|e| e.style().get_property_value("display").is_ok_and(|v| v != "none" && !v.is_empty()));
    json!({
        "shown": app_shown(),
        "signedIn": me().is_some(),
        "user": me().map(|m| json!({ "username": m.username, "name": m.name })),
        "signInPage": page,
        "menuOpen": egui::Popup::is_id_open(ctx, menu_id()),
        "linkPreviewChecked": LINK_PREVIEW_SHOWN.with(std::cell::Cell::get),
        "next": crate::continueto::from_address(),
    })
}

/// Opens the account menu as clicking your avatar does (the test hook's `account:menu`).
pub fn open_menu(ctx: &egui::Context) -> Result<(), String> {
    if me().is_none() {
        return Err("not signed in: no account menu".into());
    }
    egui::Popup::open_id(ctx, menu_id());
    ctx.request_repaint();
    Ok(())
}

/// What the avatar menu asked for (in egui memory, picked up by `Account::logic`).
pub(crate) const FACES_SETTINGS_CHANGED: &str = "starbase_faces_settings_changed";
const PHOTOS_AVAILABLE: &str = "starbase_photos_available";

fn browser_settings() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub(crate) fn photos_available() -> bool {
    browser_settings().and_then(|storage| storage.get_item(PHOTOS_AVAILABLE).ok().flatten()).as_deref() != Some("0")
}

const REQUEST: &str = "starsheets_account_request";

/// The avatars at the right of the title row: the other people in this file (left of Share),
/// drawn by `others_ui`, and yours with its menu (`me_ui`, at the far right).
pub fn me_ui(ui: &mut Ui) {
    let Some(me) = me() else { return };
    let r = avatar(ui, &me.name, &me.colour, 32.0).on_hover_text(format!("{} ({})", me.name, me.username));
    let ask = |ui: &Ui, what: &str| ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(REQUEST), what.to_string()));
    house::target(ui.ctx(), "account:avatar", r.rect);
    if r.secondary_clicked() {
        ask(ui, "settings");
    }
    // Google's account menu: who you are at the top (username, a big avatar, "Hi, name!") with
    // the main account button under it, then the rest, then Sign out.
    egui::Popup::menu(&r).id(menu_id()).show(|ui| {
        house::menu(ui, 300.0);
        house::menu_inset(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(house::S);
                house::note(ui, &me.username);
                ui.add_space(house::M);
                avatar(ui, &me.name, &me.colour, 64.0);
                ui.add_space(house::S);
                ui.label(RichText::new(format!("Hi, {}!", me.name)).size(22.0).color(theme::text()));
                ui.add_space(house::M);
                if ui.add(house::secondary("Your name and colour")).clicked() {
                    ask(ui, "profile");
                    ui.close();
                }
                ui.add_space(house::S);
            });
        });
        house::menu_sep(ui);
        // An account made through single sign-on has no password: setting one starts in "Your
        // name and colour", which first confirms who you are at the sign-in service.
        if HAS_PASSWORD.with(|p| *p.borrow()) {
            if house::menu_row(ui, Some("lock"), "Change password…", "", true) {
                ask(ui, "password");
            }
        } else if house::menu_row(ui, Some("lock"), "Set a password…", "", true) {
            ask(ui, "profile");
        }
        if house::menu_row(ui, Some(""), "Your devices…", "", true) {
            ask(ui, "devices");
        }
        if me.owner && house::menu_row(ui, Some("folder"), "Where files are kept…", "", true) {
            ask(ui, "storage");
        }
        if house::menu_row(ui, Some("settings"), "Settings…", "", true) {
            ask(ui, "settings");
        }
        if me.owner && house::menu_row(ui, Some("group"), "People…", "", true) {
            ask(ui, "people");
        }
        house::menu_sep(ui);
        if house::menu_row(ui, Some(""), "Sign out", "", true) {
            ask(ui, "signout");
        }
    });
}

/// The other people with this file open.
pub fn others_ui(ui: &mut Ui) {
    let others = crate::live::others();
    for p in others.iter().rev().take(5) {
        avatar(ui, &p.name, &p.colour, 30.0).on_hover_text(format!("{} is here", p.name));
        ui.add_space(2.0);
    }
    if others.len() > 5 {
        ui.label(RichText::new(format!("+{}", others.len() - 5)).size(house::NOTE).color(theme::text_dim()))
            .on_hover_text(others[5..].iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("\n"));
    }
}

enum Reply {
    Info {
        auth: bool,
        setup: bool,
        signup: bool,
        signup_code: bool,
        signed_in: bool,
        user: Option<Me>,
        /// Single sign-on (STARBASE_OIDC_*), when the server has it.
        sso: Option<Sso>,
    },
    /// Looked again whether this tab is still signed in (the live connection kept failing).
    Check {
        signed_in: bool,
        sso: Option<Sso>,
    },
    LinkPreview(bool),
    Signup {
        on: bool,
        code: String,
    },
    /// Signed in or out, or the password changed: start the page again.
    Reload,
    /// Signed in (a new account): go on to the link that was opened (`?next=`), else start the
    /// page again.
    SignedIn,
    /// Signed out: go to the identity provider's sign-out page (single sign-on), or reload.
    SignedOut(Option<String>),
    Saved(Me),
    /// Everyone, and (owner) whom each is linked to at the identity provider.
    People(Vec<Person>),
    /// Go to this address (the identity provider, to link an account).
    Go(String),
    Error(String),
    Done(String),
    /// Where files are kept now, and the folders that could be chosen (GET /api/storage).
    Storage(Json),
    /// The setup made the owner's account: now move to the folder picked, if any.
    SetUp,
    /// The server is restarting in its new folder: wait for it, then start the page again.
    Restarting,
    /// Backup settings and how the last one went (GET /api/backup).
    Backup(Json),
    FaceSettings(Json),
    FaceSettingsChanged(Json),
    FaceModels,
    BackupEnabled(bool),
    /// The backups kept (GET /api/backup/list).
    BackupList(Vec<Json>),
    /// Whose folders the backups hold (GET /api/backup/people).
    BackupPeople(Vec<Json>),
    BackupKey(String),
    /// Settings saved, a backup started or a restore done: say so and look again.
    BackupDone(String),
    /// The desktop apps connected to an account (GET /api/devices).
    Devices(Vec<Json>),
    /// A desktop app was removed: say so and look again.
    DeviceGone(String),
}

/// What the server says about single sign-on (GET /api/info `sso`).
#[derive(Clone, Debug, Default)]
struct Sso {
    label: String,
    /// Only single sign-on (passwords only for owners).
    only: bool,
    /// Members of the provider's owner group can set up a new server with it.
    owner_group: bool,
    /// The signed-in account is linked to a sign-in at the provider.
    linked: bool,
    /// The signed-in account has a password (linking asks for it).
    password: bool,
    /// A session made through single sign-on has ended: renew it at the provider.
    renew: bool,
    /// Signed in through single sign-on in the last 10 minutes with the provider asking who it
    /// was: an account with no password may now set one or link another sign-in.
    fresh: bool,
    /// Seconds until this session (made through single sign-on) ends.
    expires_in: Option<f64>,
}

/// Someone in People (owner): their account, their single sign-on link as (what to show, the
/// raw ids for the tooltip), and whether they have a password.
#[derive(Clone, Debug)]
struct Person {
    me: Me,
    link: Option<(String, String)>,
    password: bool,
}

fn sso_from(v: &Json) -> Option<Sso> {
    let o = v.as_object()?;
    Some(Sso {
        label: o.get("label").and_then(Json::as_str).unwrap_or("Sign in with SSO").to_string(),
        only: o.get("only").and_then(Json::as_bool).unwrap_or(false),
        owner_group: o.get("owner_group").and_then(Json::as_bool).unwrap_or(false),
        linked: o.get("linked").and_then(Json::as_bool).unwrap_or(false),
        password: o.get("password").and_then(Json::as_bool).unwrap_or(false),
        renew: o.get("renew").and_then(Json::as_bool).unwrap_or(false),
        fresh: o.get("fresh").and_then(Json::as_bool).unwrap_or(false),
        expires_in: o.get("expires_in").and_then(Json::as_f64),
    })
}

/// Goes back to the identity provider without asking (`prompt=none`) to renew a session made
/// through single sign-on (STARBASE_OIDC_SESSION_HOURS), coming back to this page (the same
/// file, which opens again from the server: callers make sure nothing is waiting to be saved).
fn renew_sso() {
    // Once: going again while the page is already on its way would restart the trip (and use up
    // the server's try limit).
    if LEAVING.with(|l| l.replace(true)) {
        return;
    }
    let Some(w) = web_sys::window() else { return };
    // Signed out with a link to go on to (`?next=`): come back there, not to this page.
    let here = crate::continueto::from_address()
        .unwrap_or_else(|| format!("{}{}", w.location().pathname().unwrap_or_default(), w.location().search().unwrap_or_default()));
    if let Some(s) = session_storage() {
        let _ = s.set_item(RENEWING, &now_ms().to_string());
    }
    crate::files::clear_unsaved_flag();
    let _ = w.location().set_href(&format!("api/oidc/login?silent=1&next={}", crate::files::enc(&here)));
}

/// The query parameter that reopens "Your name and colour" after confirming who you are at the
/// sign-in service.
const AFTER_FRESH: &str = "ss_account";

/// Goes to the sign-in service to confirm who you are (`prompt=login`), back to this page with
/// "Your name and colour" open again: an account with no password may then set one or link a
/// different sign-in for 10 minutes.
fn go_fresh_login() {
    let Some(w) = web_sys::window() else { return };
    let mut here = here();
    here.push_str(if here.contains('?') { "&" } else { "?" });
    here.push_str(AFTER_FRESH);
    here.push_str("=profile");
    crate::files::clear_unsaved_flag();
    let _ = w.location().set_href(&format!("api/oidc/login?fresh=1&next={}", crate::files::enc(&here)));
}

/// Takes `ss_account=profile` out of the address: whether it was there.
fn take_after_fresh() -> bool {
    let Some(w) = web_sys::window() else { return false };
    let Ok(search) = w.location().search() else { return false };
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else { return false };
    if params.get(AFTER_FRESH).is_none() {
        return false;
    }
    params.delete(AFTER_FRESH);
    let rest = String::from(params.to_string());
    let path = w.location().pathname().unwrap_or_default();
    if let Ok(h) = w.history() {
        let url = if rest.is_empty() { path } else { format!("{path}?{rest}") };
        let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url));
    }
    true
}

/// Whether the page came back from single sign-on with something to say (`?sso=`).
fn sso_message_waiting() -> bool {
    web_sys::window().and_then(|w| js_sys::Reflect::get(&w, &"__ssSsoMessage".into()).ok()).and_then(|v| v.as_string()).is_some_and(|m| !m.is_empty())
}

/// The people list (GET /api/users): each person, and for the owner whom they are linked to:
/// "ann@example.com at Keycloak" (what the provider called them at their last sign-in, and the
/// provider's name), with the raw ids (subject, issuer) for the tooltip.
fn people_from(v: Json) -> Reply {
    Reply::People(
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let sso = &p["sso"];
                let link = sso["subject"].as_str().map(|sub| {
                    let iss = sso["issuer"].as_str().unwrap_or("");
                    let provider = sso["provider"].as_str().filter(|s| !s.is_empty()).unwrap_or(iss);
                    let shown = match sso["who"].as_str().filter(|s| !s.is_empty()) {
                        Some(who) => format!("{who} at {provider}"),
                        None => format!("an account at {provider}"),
                    };
                    (shown, format!("Subject: {sub}\nIssuer: {iss}"))
                });
                let password = p["password"].as_bool().unwrap_or(true);
                me_from(p).map(|me| Person { me, link, password })
            })
            .collect(),
    )
}

/// Sends the browser to the server's single sign-on, coming back to this page.
fn go_sso_login() {
    let Some(w) = web_sys::window() else { return };
    let here = crate::continueto::from_address()
        .unwrap_or_else(|| format!("{}{}", w.location().pathname().unwrap_or_default(), w.location().search().unwrap_or_default()));
    crate::files::clear_unsaved_flag();
    let _ = w.location().set_href(&format!("api/oidc/login?next={}", crate::files::enc(&here)));
}

/// This page's address, to come back to after linking.
fn here() -> String {
    web_sys::window()
        .map_or_else(String::new, |w| format!("{}{}", w.location().pathname().unwrap_or_default(), w.location().search().unwrap_or_default()))
}

/// What happened at the last single sign-on (index.html reads `?sso=` from the address and
/// explains it), taken once.
fn sso_message() -> Option<String> {
    let w = web_sys::window()?;
    let v = js_sys::Reflect::get(&w, &"__ssSsoMessage".into()).ok()?.as_string().filter(|m| !m.is_empty())?;
    let _ = js_sys::Reflect::set(&w, &"__ssSsoMessage".into(), &"".into());
    Some(v)
}

#[derive(Default, PartialEq)]
enum Dialog {
    #[default]
    None,
    SignIn,
    SignUp,
    Setup,
    Profile,
    Password,
    People,
    Storage,
    Settings,
    Devices,
}

#[derive(Default)]
pub struct Account {
    inbox: Rc<RefCell<Vec<Reply>>>,
    booted: bool,
    auth: bool,
    /// The server lets people make their own account (and whether that needs a code).
    signup: bool,
    signup_code: bool,
    sso: Option<Sso>,
    /// The owner's sign-up setting, as shown in People.
    signup_on: bool,
    signup_secret: String,
    link_preview_on: bool,
    dialog: Dialog,
    busy: bool,
    error: Option<String>,
    done: Option<String>,
    focused: bool,
    /// The HTML sign-in form (index.html) is showing.
    dom_sign_in: bool,
    // Form fields.
    code: String,
    username: String,
    name: String,
    password: String,
    password2: String,
    current: String,
    colour: String,
    people: Vec<Person>,
    remove_armed: Option<String>,
    /// Renewing a single sign-on session while the page stays open: when to renew before it
    /// ends (ms; None: not a single sign-on session, or a renewal before the end just failed),
    /// since when a renewal is waiting for a quiet moment, when the last input came, and when
    /// the sign-in was last checked for the live connection.
    renew_at: Option<f64>,
    renew_waiting: Option<f64>,
    last_input: f64,
    last_check: f64,
    /// A renewal before the session ended didn't work (the provider wanted to ask something):
    /// no new try before it ends.
    renew_held: bool,
    /// Came back from confirming who you are: open "Your name and colour" once signed in.
    after_fresh: bool,
    /// Where files are kept (dialog and first-time setup): what the server said, the folder
    /// picked ("" = the top folder), whether to switch to StarSheets files already there, and
    /// when the restart began (waiting for the server to answer again).
    storage: Option<Json>,
    storage_pick: Option<String>,
    storage_use: bool,
    /// First-time setup: the setup code the folder list was last asked for with (the server
    /// lists folders only to someone with the code).
    storage_code: String,
    restarting: Option<f64>,
    polled: f64,
    /// Backups (owner): what the server said, the backups kept (None = loading, or not set up),
    /// the form (folder, daily on/off, hour), the recovery key when shown, a key typed to use
    /// backups made before, and the backup a "Restore" was clicked on once.
    settings_tab: usize,
    face_settings: Option<Json>,
    backup: Option<Json>,
    backup_list: Option<Vec<Json>>,
    backup_to: String,
    backup_on: bool,
    backup_hour: usize,
    backup_key: Option<String>,
    backup_old_key: String,
    backup_has_old: bool,
    backup_armed: Option<String>,
    backup_polled: f64,
    /// Whose files a restore brings back (owner, sign-in on): the people the backups hold
    /// ({name, account}), and the choice (0 = your own, then each other person, then everyone).
    backup_people: Vec<Json>,
    backup_whose: usize,
    /// Your devices (or, for the owner from People, someone else's): whose ("" = your own), the
    /// list (None = loading) and the one a "Remove" was clicked on once.
    devices_of: String,
    devices: Option<Vec<Json>>,
    device_armed: Option<String>,
}

fn me_from(v: &Json) -> Option<Me> {
    let u = v.as_object()?;
    let s = |k: &str| u.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    Some(Me { username: s("username"), name: s("name"), colour: s("colour"), owner: u.get("owner").and_then(Json::as_bool).unwrap_or(false) })
}

/// Sends a JSON request; `then` turns a good answer into a Reply, errors become Reply::Error.
fn call(
    inbox: &Rc<RefCell<Vec<Reply>>>,
    ctx: &egui::Context,
    method: Method,
    url: &str,
    body: Option<Json>,
    then: impl FnOnce(Json) -> Reply + 'static,
) {
    let (inbox, ctx, url) = (inbox.clone(), ctx.clone(), url.to_string());
    wasm_bindgen_futures::spawn_local(async move {
        let body = body.map(|b| serde_json::to_vec(&b).unwrap_or_default());
        let reply = match fetch(method, &url, body, None).await {
            Ok((s, _, bytes)) => {
                let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
                if s < 400 { then(v) } else { Reply::Error(crate::home::said(&v, s)) }
            }
            Err(e) => Reply::Error(e),
        };
        inbox.borrow_mut().push(reply);
        ctx.request_repaint();
    });
}

/// How long ago, in words ("just now", "5 minutes ago", "3 days ago").
fn ago(secs: f64) -> String {
    let s = secs.max(0.0);
    let (n, unit) = if s < 120.0 {
        return "just now".into();
    } else if s < 7200.0 {
        ((s / 60.0) as u64, "minute")
    } else if s < 172_800.0 {
        ((s / 3600.0) as u64, "hour")
    } else {
        ((s / 86_400.0) as u64, "day")
    };
    format!("{n} {unit}s ago")
}

fn link_preview_reply(v: Json) -> Reply {
    Reply::LinkPreview(v["on"].as_bool().unwrap_or(false))
}

fn signup_reply(v: Json) -> Reply {
    Reply::Signup { on: v["on"].as_bool().unwrap_or(false), code: v["code"].as_str().unwrap_or("").to_string() }
}

fn reload() {
    crate::files::clear_unsaved_flag();
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}

impl Account {
    pub fn logic(&mut self, ctx: &egui::Context) {
        if !self.booted {
            self.booted = true;
            self.after_fresh = take_after_fresh();
            // The sign-in form (index.html) goes on to this after signing in.
            if let Some(w) = web_sys::window() {
                let next = crate::continueto::from_address().map_or(wasm_bindgen::JsValue::NULL, |n| n.into());
                let _ = js_sys::Reflect::set(&w, &"__ssNext".into(), &next);
            }
            self.last_input = now_ms();
            call(&self.inbox, ctx, Method::Get, "api/info", None, |v| Reply::Info {
                auth: v["auth"].as_bool().unwrap_or(false),
                setup: v["setup"].as_bool().unwrap_or(false),
                signup: v["signup"].as_bool().unwrap_or(false),
                signup_code: v["signup_code"].as_bool().unwrap_or(false),
                signed_in: v["signed_in"].as_bool().unwrap_or(true),
                user: me_from(&v["user"]),
                sso: sso_from(&v["sso"]),
            });
        }
        let replies: Vec<Reply> = std::mem::take(&mut *self.inbox.borrow_mut());
        for r in replies {
            self.busy = false;
            match r {
                Reply::Info { auth, setup, signup, signup_code, signed_in, user, sso } => {
                    self.auth = auth;
                    self.signup = signup;
                    self.signup_code = signup_code;
                    self.sso = sso;
                    ME.with(|m| *m.borrow_mut() = user);
                    if !auth || signed_in {
                        // Signed in already: go straight to `?next=` (a path on this server, without a `next=` of its own, so it can't go round in circles), else just drop it.
                        if auth
                            && signed_in
                            && let Some(next) = crate::continueto::from_address().filter(|n| !crate::continueto::loops(n))
                            && let Some(w) = web_sys::window()
                        {
                            let _ = w.location().replace(&next);
                            return;
                        }
                        APP_SHOWN.with(|s| s.set(true));
                        crate::continueto::forget();
                    }
                    HAS_PASSWORD.with(|p| *p.borrow_mut() = self.sso.as_ref().is_none_or(|s| s.password || !signed_in));
                    // Back from a renewal started before the session ended: if the provider
                    // couldn't do it without asking, the session simply carries on until it
                    // ends (then the sign-in box says so); no error now, and no new try.
                    let renewed = renewing_since().is_some_and(|t| now_ms() - t < 180_000.0);
                    if let Some(s) = session_storage() {
                        let _ = s.remove_item(RENEWING);
                    }
                    self.renew_at = None;
                    if auth && signed_in && renewed && sso_message().is_some() {
                        log::warn!("single sign-on: renewing before the session ended didn't work; it lasts until it ends");
                        self.renew_held = true;
                    } else {
                        self.plan_renewal(signed_in);
                    }
                    // Back from linking an account at the provider: say how it went.
                    if auth
                        && signed_in
                        && let Some(why) = sso_message()
                    {
                        self.open(Dialog::Profile);
                        self.error = Some(why);
                    }
                    // Back from confirming who you are: "Your name and colour" again.
                    if auth && signed_in && std::mem::take(&mut self.after_fresh) {
                        self.open(Dialog::Profile);
                    }
                    // A session made through single sign-on has ended: renew it at the provider
                    // (unless that just failed: then the sign-in box says so).
                    if auth && !signed_in && !setup && self.sso.as_ref().is_some_and(|s| s.renew) && !sso_message_waiting() {
                        renew_sso();
                        return;
                    }
                    if auth && !signed_in {
                        self.open(if setup { Dialog::Setup } else { Dialog::SignIn });
                        if setup {
                            self.load_storage(ctx);
                        }
                    }
                }
                Reply::Check { signed_in, sso } => {
                    if signed_in {
                        // Still signed in (the server or the network was away): carry on.
                        self.sso = sso.or(self.sso.take());
                        self.plan_renewal(true);
                    } else if sso.as_ref().is_some_and(|s| s.renew) {
                        renewal_needed();
                    } else if self.auth {
                        ask_sign_in();
                    }
                }
                Reply::Reload => reload(),
                Reply::SignedIn => match crate::continueto::from_address() {
                    Some(next) => {
                        if let Some(w) = web_sys::window() {
                            crate::files::clear_unsaved_flag();
                            let _ = w.location().replace(&next);
                        }
                    }
                    None => reload(),
                },
                Reply::SignedOut(to) => match (to, web_sys::window()) {
                    (Some(url), Some(w)) => {
                        crate::files::clear_unsaved_flag();
                        let _ = w.location().set_href(&url);
                    }
                    _ => reload(),
                },
                Reply::Saved(me) => {
                    ME.with(|m| *m.borrow_mut() = Some(me));
                    self.dialog = Dialog::None;
                }
                Reply::People(p) => self.people = p,
                Reply::Go(url) => {
                    if let Some(w) = web_sys::window() {
                        crate::files::clear_unsaved_flag();
                        let _ = w.location().set_href(&url);
                    }
                }
                Reply::Storage(v) => {
                    self.storage_pick.get_or_insert_with(|| v["current"].as_str().unwrap_or("").to_string());
                    self.storage = Some(v);
                }
                Reply::SetUp => match self.storage_pick.clone().filter(|p| !p.is_empty()) {
                    Some(folder) => call(&self.inbox, ctx, Method::Post, "api/storage", Some(json!({ "folder": folder, "mode": "move" })), |v| {
                        if v["restarting"].as_bool().unwrap_or(false) { Reply::Restarting } else { Reply::Reload }
                    }),
                    None => reload(),
                },
                Reply::Backup(v) => {
                    let was_running = self.backup.as_ref().is_some_and(|b| b["running"].as_bool().unwrap_or(false));
                    let first = self.backup.is_none();
                    if first {
                        self.backup_to = v["to"].as_str().filter(|t| !t.is_empty()).or(v["suggest"].as_str()).unwrap_or("").to_string();
                        self.backup_on = v["on"].as_bool().unwrap_or(true) || !v["set_up"].as_bool().unwrap_or(false);
                        self.backup_hour = v["hour"].as_u64().unwrap_or(3).min(23) as usize;
                    }
                    let set_up = v["set_up"].as_bool().unwrap_or(false);
                    let finished = was_running && !v["running"].as_bool().unwrap_or(false);
                    self.backup = Some(v);
                    if finished {
                        // The line at the top now says how it went.
                        self.done = None;
                    }
                    if set_up && (first || finished) {
                        call(&self.inbox, ctx, Method::Get, "api/backup/list", None, |v| {
                            Reply::BackupList(v.as_array().cloned().unwrap_or_default())
                        });
                        if self.auth {
                            call(&self.inbox, ctx, Method::Get, "api/backup/people", None, |v| {
                                Reply::BackupPeople(v.as_array().cloned().unwrap_or_default())
                            });
                        }
                    }
                }
                Reply::FaceSettings(v) => self.face_settings = Some(v),
                Reply::FaceSettingsChanged(v) => {
                    self.face_settings = Some(v);
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(FACES_SETTINGS_CHANGED), true));
                }
                Reply::FaceModels => call(&self.inbox, ctx, Method::Get, "api/photos/faces", None, Reply::FaceSettings),
                Reply::BackupEnabled(enabled) => {
                    if let Some(v) = &mut self.backup {
                        v["enabled"] = json!(enabled);
                    }
                    self.done = Some(if enabled { "Backups enabled." } else { "Backups turned off." }.into());
                }
                Reply::BackupList(l) => self.backup_list = Some(l),
                Reply::BackupPeople(p) => self.backup_people = p,
                Reply::BackupKey(k) => self.backup_key = Some(k),
                Reply::BackupDone(msg) => {
                    self.done = Some(msg);
                    self.error = None;
                    self.backup_old_key.clear();
                    self.backup_has_old = false;
                    call(&self.inbox, ctx, Method::Get, "api/backup", None, Reply::Backup);
                    call(&self.inbox, ctx, Method::Get, "api/backup/list", None, |v| Reply::BackupList(v.as_array().cloned().unwrap_or_default()));
                }
                Reply::Restarting => {
                    self.dialog = Dialog::None;
                    self.restarting = Some(ctx.input(|i| i.time));
                }
                Reply::LinkPreview(on) => self.link_preview_on = on,
                Reply::Signup { on, code } => {
                    self.signup_on = on;
                    self.signup_secret = code;
                }
                Reply::Devices(list) => self.devices = Some(list),
                Reply::DeviceGone(msg) => {
                    self.done = Some(msg);
                    self.error = None;
                    self.load_devices(ctx);
                }
                Reply::Error(e) => self.error = Some(e),
                Reply::Done(msg) => {
                    self.done = Some(msg);
                    self.error = None;
                    self.name.clear();
                    self.username.clear();
                    self.password.clear();
                    call(&self.inbox, ctx, Method::Get, "api/users", None, people_from);
                }
            }
        }
        if let Some(req) = ctx.data_mut(|m| m.remove_temp::<String>(egui::Id::unique(REQUEST))) {
            match req.as_str() {
                "profile" => self.open(Dialog::Profile),
                "password" => self.open(Dialog::Password),
                "storage" => {
                    self.open(Dialog::Storage);
                    self.load_storage(ctx);
                }
                "settings" | "photos-settings" | "backups" => {
                    self.open(Dialog::Settings);
                    let owner = me().is_some_and(|m| m.owner);
                    self.settings_tab = match req.as_str() {
                        "photos-settings" => 1,
                        "backups" if owner => 2,
                        _ => 0,
                    };
                    self.face_settings = None;
                    call(&self.inbox, ctx, Method::Get, "api/photos/faces", None, Reply::FaceSettings);
                    self.backup = None;
                    self.backup_list = None;
                    self.backup_key = None;
                    self.backup_armed = None;
                    self.backup_people.clear();
                    self.backup_whose = 0;
                    if owner {
                        call(&self.inbox, ctx, Method::Get, "api/backup", None, Reply::Backup);
                    }
                }
                "devices" => {
                    self.open(Dialog::Devices);
                    self.devices_of.clear();
                    self.load_devices(ctx);
                }
                "people" => {
                    self.open(Dialog::People);
                    call(&self.inbox, ctx, Method::Get, "api/users", None, people_from);
                    call(&self.inbox, ctx, Method::Get, "api/signup/settings", None, signup_reply);
                    call(&self.inbox, ctx, Method::Get, "api/link-preview/settings", None, link_preview_reply);
                }
                "signout" => call(&self.inbox, ctx, Method::Post, "api/logout", None, |v| {
                    Reply::SignedOut(v["logout_url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://")).map(str::to_string))
                }),
                "sso-unlink" => call(&self.inbox, ctx, Method::Post, "api/oidc/unlink", None, |_| Reply::Reload),
                _ => {}
            }
        }
        self.renew_in_background(ctx);
        let leaving = LEAVING.with(|l| l.get());
        if SIGN_IN.with(|s| std::mem::take(&mut *s.borrow_mut())) && !leaving && !matches!(self.dialog, Dialog::Setup | Dialog::SignUp) {
            self.open(Dialog::SignIn);
        }
    }

    /// When to renew a single sign-on session before it ends: RENEW_LEAD before (or now, if it
    /// ends sooner than that). Not for a password session, nor after a renewal that didn't work.
    fn plan_renewal(&mut self, signed_in: bool) {
        /// How long before the end a renewal is tried (it then waits for a quiet moment).
        const RENEW_LEAD: f64 = 600.0;
        self.renew_at = match self.sso.as_ref().and_then(|s| s.expires_in) {
            Some(left) if signed_in && !self.renew_held => Some(now_ms() + (left - RENEW_LEAD).max(0.0) * 1000.0),
            _ => None,
        };
    }

    /// Renews a single sign-on session while the page stays open: shortly before it ends, or
    /// when a request found it ended (files.rs `fetch`, or the live connection can't get back
    /// in). It goes to the provider without asking and comes back to the same page, so it waits
    /// for a quiet moment: nothing waiting to be saved or on its way, no cell or text field being
    /// typed in, no dialog open, and no input for a few seconds. If changes can't be saved for a minute
    /// because the session has ended, the sign-in box is shown instead (as before).
    fn renew_in_background(&mut self, ctx: &egui::Context) {
        let now = now_ms();
        if ctx.input(|i| !i.events.is_empty() || i.pointer.any_down()) {
            self.last_input = now;
        }
        // The live connection can't get back in: is the sign-in still good?
        if CHECK.with(|c| std::mem::take(&mut *c.borrow_mut())) && self.auth && now - self.last_check > 20_000.0 {
            self.last_check = now;
            call(&self.inbox, ctx, Method::Get, "api/info", None, |v| Reply::Check {
                signed_in: v["signed_in"].as_bool().unwrap_or(true),
                sso: sso_from(&v["sso"]),
            });
        }
        let ended = RENEW.with(|r| *r.borrow());
        let due = self.renew_at.is_some_and(|t| now >= t);
        if !ended && !due {
            self.renew_waiting = None;
            if let Some(t) = self.renew_at {
                ctx.request_repaint_after(std::time::Duration::from_millis((t - now).clamp(1000.0, 3_600_000.0) as u64));
            }
            return;
        }
        if ended {
            // Ended: the sign-in box isn't needed (yet).
            SIGN_IN.with(|s| *s.borrow_mut() = false);
        }
        let since = *self.renew_waiting.get_or_insert(now);
        let unsaved = web_sys::window().and_then(|w| js_sys::Reflect::get(&w, &"__ssUnsaved".into()).ok()).is_some_and(|v| v.is_truthy());
        let typing = TYPING.with(|t| *t.borrow());
        // Any text field with focus (share, rename, new folder ...) or any dialog open: the
        // renewal leaves the page, so what was typed there would be lost.
        let busy = ctx.memory(|m| m.focused().is_some() || m.top_modal_layer().is_some());
        let quiet = !unsaved
            && !busy
            && !crate::files::writes_in_flight()
            && !typing
            && self.dialog == Dialog::None
            && !self.dom_sign_in
            && now - self.last_input > 4000.0
            && !ctx.input(|i| i.pointer.any_down());
        if quiet {
            RENEW.with(|r| *r.borrow_mut() = false);
            self.renew_at = None;
            renew_sso();
            return;
        }
        if ended && unsaved && now - since > 60_000.0 {
            // Changes can't be saved while the session has ended, and they would be lost by
            // leaving the page now: say so with the sign-in box, as before.
            RENEW.with(|r| *r.borrow_mut() = false);
            self.renew_waiting = None;
            self.open(Dialog::SignIn);
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(1000));
    }

    fn open(&mut self, d: Dialog) {
        if self.dialog == d {
            return;
        }
        self.dialog = d;
        self.error = None;
        self.done = None;
        self.focused = false;
        self.password.clear();
        self.password2.clear();
        self.current.clear();
        self.remove_armed = None;
        self.device_armed = None;
        // Your own name only in "Your name and colour": People's "Their name" shares the field.
        match me().filter(|_| self.dialog == Dialog::Profile) {
            Some(me) => {
                self.name = me.name;
                self.colour = me.colour;
            }
            None if self.dialog == Dialog::People => {
                self.name.clear();
                self.username.clear();
            }
            None => {}
        }
    }

    fn field(ui: &mut Ui, text: &mut String, hint: &str, password: bool) -> egui::Response {
        ui.add(
            egui::TextEdit::singleline(text).password(password).hint_text(hint).desired_width(f32::INFINITY).margin(vec2(house::M, house::S + 1.0)),
        )
    }

    fn messages(&self, ui: &mut Ui) {
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
        }
        if let Some(d) = &self.done {
            ui.label(RichText::new(d).size(house::NOTE).color(house::GREEN));
        }
    }

    /// Focuses the first box once (grabbing it every frame swallows Enter).
    fn first(&mut self, r: &egui::Response) {
        if !self.focused {
            r.request_focus();
            self.focused = true;
        }
    }

    fn enter(ui: &Ui, r: &egui::Response) -> bool {
        (r.lost_focus() || r.has_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter))
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        self.sign_in_dom(ctx);
        match self.dialog {
            Dialog::None => {}
            Dialog::SignIn => {}
            Dialog::SignUp => self.sign_up(ctx),
            Dialog::Setup => self.setup(ctx),
            Dialog::Profile => self.profile(ctx),
            Dialog::Password => self.change_password(ctx),
            Dialog::People => self.people(ctx),
            Dialog::Storage => self.storage_ui(ctx),
            Dialog::Settings => self.settings_ui(ctx),
            Dialog::Devices => self.devices_ui(ctx),
        }
        self.restarting_ui(ctx);
    }

    fn load_storage(&mut self, ctx: &egui::Context) {
        self.storage = None;
        self.storage_pick = None;
        self.storage_use = false;
        call(&self.inbox, ctx, Method::Get, "api/storage", None, Reply::Storage);
    }

    /// Where files are kept (owner): the folder in use, and a list of the folders the server can
    /// reach to choose another. Files move there (or, if StarSheets files are already there,
    /// StarSheets can switch to those); the server then restarts.
    fn storage_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut cancel = false;
        let mut go = false;
        let v = self.storage.clone();
        house::dialog(ctx, "starsheets_storage", 540.0, "Where files are kept", Some(&mut close), |ui| {
            let Some(v) = &v else {
                ui.label(RichText::new("Loading…").color(theme::text_dim()));
                return;
            };
            let location = v["location"].as_str().unwrap_or("the server");
            let current = v["current"].as_str().unwrap_or("");
            let here = |p: &str| if p.is_empty() { location.to_string() } else { format!("{location} › {}", p.replace('/', " › ")) };
            house::text(ui, &format!("Now: {}", here(current)));
            house::note(
                ui,
                "Choose another folder below. These are the folders the server can reach; to offer others, give the server a bigger folder when it's installed.",
            );
            house::gap(ui);
            let folders: Vec<String> = v["folders"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
            let with_files: Vec<String> = v["has_files"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
            let pick = self.storage_pick.clone().unwrap_or_default();
            egui::Frame::NONE.stroke(egui::Stroke::new(1.0, theme::divider())).corner_radius(8).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(280.0).min_scrolled_height(280.0).auto_shrink([false, false]).show(ui, |ui| {
                    for f in std::iter::once(String::new()).chain(folders.iter().cloned()) {
                        let depth = if f.is_empty() { 0 } else { f.matches('/').count() + 1 };
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                        let p = ui.painter().clone();
                        if pick == f {
                            p.rect_filled(r, 6, theme::SHARE);
                        } else if resp.hovered() {
                            p.rect_filled(r, 6, theme::menu_hover());
                        }
                        let x = r.left() + 8.0 + depth as f32 * 18.0;
                        if let Some(img) = crate::icons::icon("folder") {
                            img.paint_at(ui, egui::Rect::from_center_size(egui::pos2(x + 10.0, r.center().y), vec2(18.0, 18.0)));
                        }
                        let mut label = if f.is_empty() { format!("{location} (the top)") } else { f.rsplit('/').next().unwrap_or(&f).to_string() };
                        if f == current {
                            label.push_str("  · in use");
                        } else if with_files.contains(&f) || (f.is_empty() && v["top_has_files"].as_bool().unwrap_or(false)) {
                            label.push_str("  · has Starbase files");
                        }
                        p.text(egui::pos2(x + 28.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), theme::text());
                        if resp.clicked() {
                            self.storage_pick = Some(f.clone());
                            self.storage_use = with_files.contains(&f) || (f.is_empty() && v["top_has_files"].as_bool().unwrap_or(false));
                        }
                    }
                });
            });
            let target_has = with_files.contains(&pick) || (pick.is_empty() && v["top_has_files"].as_bool().unwrap_or(false));
            if pick != current {
                house::gap(ui);
                if target_has {
                    ui.radio_value(
                        &mut self.storage_use,
                        true,
                        "Use the Starbase files already in that folder (accounts there too: you may need to sign in again)",
                    );
                    ui.radio_value(&mut self.storage_use, false, "Move my files there");
                } else {
                    house::note(
                        ui,
                        "Your spreadsheets, folders, versions, bin and accounts move there. Nobody can have a spreadsheet open while it happens; Starbase restarts afterwards (a few seconds).",
                    );
                }
            }
            self.messages(ui);
            match house::actions(ui, &["Cancel", "Use this folder"]) {
                Some(0) => cancel = true,
                Some(_) => go = pick != current,
                None => {}
            }
        });
        if close || cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = Dialog::None;
        } else if go && !self.busy {
            self.busy = true;
            self.error = None;
            let body = json!({ "folder": self.storage_pick.clone().unwrap_or_default(), "mode": if self.storage_use { "use" } else { "move" } });
            call(&self.inbox, ctx, Method::Post, "api/storage", Some(body), |v| {
                if v["restarting"].as_bool().unwrap_or(false) { Reply::Restarting } else { Reply::Reload }
            });
        }
    }

    /// Backups (owner): where they go, once a day at what hour, the recovery key, how the last
    /// one went, "Back up now", and the backups kept, each with "Restore" (copies your files from
    /// then into a new folder; nothing is overwritten). See the server's backup.rs.
    fn settings_tabs(&mut self, ui: &mut Ui) -> bool {
        let labels: &[&str] = if me().is_some_and(|m| m.owner) { &["Appearance", "Photos", "Backups"] } else { &["Appearance", "Photos"] };
        if self.settings_tab >= labels.len() {
            self.settings_tab = 0;
        }
        let changed = house::tabs(ui, &mut self.settings_tab, labels);
        house::gap(ui);
        changed
    }

    fn settings_ui(&mut self, ctx: &egui::Context) {
        if self.settings_tab == 2 && me().is_some_and(|m| m.owner) {
            self.backups_ui(ctx);
            return;
        }
        let mut close = false;
        house::dialog(ctx, "starsheets_settings", 560.0, "Settings", Some(&mut close), |ui| {
            if self.settings_tabs(ui) {
                if self.settings_tab == 1 {
                    call(&self.inbox, ctx, Method::Get, "api/photos/faces", None, Reply::FaceSettings);
                }
                return;
            }
            if self.settings_tab == 0 {
                let mut dark = crate::theme::drive_dark();
                let r = ui.checkbox(&mut dark, "Dark appearance in Drive");
                house::target(ctx, "settings:dark", r.rect);
                if r.changed() {
                    crate::theme::set_drive_dark(dark);
                }
                house::note(ui, "This setting is saved in this browser. Photo Mode always uses the dark appearance.");
            } else {
                self.photos_settings_ui(ui, ctx);
            }
            self.messages(ui);
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = Dialog::None;
        }
    }

    fn photos_settings_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut enabled = photos_available();
        let r = ui.checkbox(&mut enabled, "Show Photo Mode in Drive on this browser");
        house::target(ctx, "settings:photos-enabled", r.rect);
        if r.changed()
            && let Some(storage) = browser_settings()
        {
            let _ = storage.set_item(PHOTOS_AVAILABLE, if enabled { "1" } else { "0" });
        }
        let Some(state) = &self.face_settings else {
            house::note(ui, "Loading face recognition settings…");
            return;
        };
        let mut on = state["on"].as_bool().unwrap_or(false);
        let busy = state["download"]["busy"].as_bool() == Some(true);
        let r = ui.add_enabled(state["can_toggle"].as_bool() == Some(true) && !self.busy && !busy, egui::Checkbox::new(&mut on, "Face recognition"));
        house::target(ctx, "settings:faces", r.rect);
        if r.changed() {
            self.busy = true;
            self.error = None;
            call(&self.inbox, ctx, Method::Post, "api/photos/faces", Some(json!({"on": on})), Reply::FaceSettingsChanged);
        }
        if state["can_download"].as_bool() == Some(true)
            && ui.add_enabled(!self.busy && !busy, house::secondary("Download face models")).clicked()
        {
            self.busy = true;
            call(&self.inbox, ctx, Method::Post, "api/photos/faces/models", Some(json!({})), |_| Reply::FaceModels);
        }
        if busy {
            house::note(ui, "Downloading face models…");
            if ctx.input(|i| i.time) - self.backup_polled > 2.0 {
                self.backup_polled = ctx.input(|i| i.time);
                call(&self.inbox, ctx, Method::Get, "api/photos/faces", None, Reply::FaceSettings);
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
        house::note(ui, state["note"].as_str().unwrap_or("Turning off face recognition keeps names and groups."));
    }

    fn backups_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut cancel = false;
        let mut save = false;
        let mut now = false;
        let mut restore = None;
        let mut master = None;
        let mut show_key = false;
        let Some(v) = self.backup.clone() else {
            house::dialog(ctx, "starsheets_settings", 560.0, "Settings", Some(&mut close), |ui| {
                if self.settings_tabs(ui) {
                    return;
                }
                ui.label(RichText::new("Loading…").color(theme::text_dim()));
                self.messages(ui);
            });
            if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.dialog = Dialog::None;
            }
            return;
        };
        let set_up = v["set_up"].as_bool().unwrap_or(false);
        let running = v["running"].as_bool().unwrap_or(false);
        // While one runs, look again every two seconds.
        let t = ctx.input(|i| i.time);
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            if t - self.backup_polled > 2.0 {
                self.backup_polled = t;
                call(&self.inbox, ctx, Method::Get, "api/backup", None, Reply::Backup);
            }
        }
        let (list, key) = (self.backup_list.clone(), self.backup_key.clone());
        // "Whose files" (sign-in on): your own, each other person the backups hold, everyone's.
        let mine = me().map(|m| m.username).unwrap_or_default();
        let mut whose: Vec<(String, Option<String>)> = vec![("Your own files".into(), None)];
        if self.auth && !self.backup_people.is_empty() {
            for p in &self.backup_people {
                let name = p["name"].as_str().unwrap_or("");
                if name != mine && !name.is_empty() {
                    let label = if p["account"].as_bool().unwrap_or(false) { name.to_string() } else { format!("{name} (no account now)") };
                    whose.push((label, Some(name.to_string())));
                }
            }
            whose.push(("Everyone's files".into(), Some(String::new())));
        }
        if self.backup_whose >= whose.len() {
            self.backup_whose = 0;
        }
        let when = |iso: &str| format!("{} UTC", iso.replace('T', " ").chars().take(16).collect::<String>());
        house::dialog(ctx, "starsheets_settings", 560.0, "Settings", Some(&mut close), |ui| {
            if self.settings_tabs(ui) {
                return;
            }
            let enabled = v["enabled"].as_bool().unwrap_or(true);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.busy, |ui| {
                    let (track, response) = ui.allocate_exact_size(vec2(42.0, 24.0), Sense::click());
                    let fill = if enabled { theme::BLUE } else { theme::text_dim() };
                    ui.painter().rect_filled(track, 12.0, fill);
                    let x = if enabled { track.right() - 12.0 } else { track.left() + 12.0 };
                    ui.painter().circle_filled(egui::pos2(x, track.center().y), 8.0, Color32::WHITE);
                    if response.has_focus() || response.hovered() {
                        ui.painter().rect_stroke(track.expand(2.0), 14.0, egui::Stroke::new(1.0, fill), egui::StrokeKind::Outside);
                    }
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), enabled, "Enable backups"));
                    house::target(ctx, "settings:backups-enabled", track);
                    if response.clicked() {
                        master = Some(!enabled);
                    }
                });
                house::text(ui, if enabled { "Backups on" } else { "Backups off" });
            });
            if !enabled {
                house::note(ui, "Scheduled and manual backups are off. Existing backups can still be restored.");
            }
            house::note(
                ui,
                "Starbase can back up files, versions, the bin and accounts daily to a folder you choose: best a disk or NAS share that isn't the one your files are on. Backups are locked with a recovery key.",
            );
            house::gap(ui);
            if set_up {
                let last = &v["last"];
                if running {
                    house::text(ui, "Backing up now…");
                } else if let Some(w) = last["when"].as_str() {
                    let ok = last["ok"].as_bool().unwrap_or(false);
                    let msg = last["message"].as_str().unwrap_or("");
                    let line = if ok { format!("Last backup: {} · {msg}", when(w)) } else { format!("The last backup failed ({}): {msg}", when(w)) };
                    ui.label(RichText::new(line).size(house::BODY).color(if ok { house::GREEN } else { house::ERROR }));
                } else {
                    house::text(ui, "No backup made yet.");
                }
                house::gap(ui);
            }
            house::label(
                ui,
                "Backup folder",
                "The whole path, as the server sees it (with Docker: a disk or share mounted into the container, such as /backups). Empty the first time.",
            );
            Self::field(ui, &mut self.backup_to, "/backups", false);
            ui.add_space(house::S);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.backup_on, "Back up every day at");
                let hours: Vec<String> = (0..24).map(|h| format!("{h:02}:00")).collect();
                let labels: Vec<&str> = hours.iter().map(String::as_str).collect();
                house::select(ui, "backup_hour", 96.0, &mut self.backup_hour, &labels, &[]);
                house::note(ui, &format!("server time ({})", v["timezone"].as_str().unwrap_or("UTC")));
            });
            if !set_up || self.backup_has_old {
                ui.add_space(house::S);
                ui.checkbox(&mut self.backup_has_old, "That folder has backups from before: use my recovery key");
                if self.backup_has_old {
                    ui.add_space(house::S);
                    Self::field(ui, &mut self.backup_old_key, "Recovery key (xxxx-xxxx-…)", false);
                }
            }
            if set_up {
                house::gap(ui);
                house::label(
                    ui,
                    "Recovery key",
                    "Backups can't be opened without it, and it is kept on this server: if the server is lost, so is the key. Keep a copy somewhere else (a password manager, or printed).",
                );
                match &key {
                    Some(k) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(k).monospace().size(house::BODY).color(theme::text()));
                            if ui.add(house::link("Copy")).clicked() {
                                ui.ctx().copy_text(k.clone());
                                self.done = Some("Recovery key copied.".into());
                            }
                            if ui.add(house::link("Download")).clicked() {
                                let text = format!(
                                    "Starbase backup recovery key\n\n{k}\n\nBackup folder: {}\nThe backups are a restic repository: restic or rustic can open them with this key as the password.\n",
                                    self.backup_to
                                );
                                if let Err(e) = crate::web::download_file("Starbase backup recovery key.txt", text.as_bytes()) {
                                    self.error = Some(e);
                                }
                            }
                        });
                    }
                    None => {
                        if ui.add(house::link("Show recovery key")).clicked() {
                            show_key = true;
                        }
                    }
                }
                house::gap(ui);
                house::label(
                    ui,
                    "Backups kept",
                    "The last 7 days, 4 weeks and 12 months. Restore copies files from then into a new folder; nothing is overwritten.",
                );
                if whose.len() > 1 {
                    ui.horizontal(|ui| {
                        house::text(ui, "Whose files");
                        let labels: Vec<&str> = whose.iter().map(|(l, _)| l.as_str()).collect();
                        house::select(ui, "backup_whose", 260.0, &mut self.backup_whose, &labels, &[0, labels.len() - 2]);
                    });
                    house::note(ui, "Someone else's come back beside their own folder; if they have no account here, into yours, named after them.");
                }
                match &list {
                    None => house::note(ui, "Loading…"),
                    Some(l) if l.is_empty() => house::note(ui, "None yet."),
                    Some(l) => {
                        egui::ScrollArea::vertical().max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
                            for b in l {
                                let id = b["id"].as_str().unwrap_or("").to_string();
                                ui.horizontal(|ui| {
                                    house::text(ui, &format!("{} UTC", b["label"].as_str().unwrap_or("").replace('.', ":")));
                                    house::note(ui, &format!("{} files · {}", b["files"].as_u64().unwrap_or(0), b["size"].as_str().unwrap_or("")));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let armed = self.backup_armed.as_deref() == Some(id.as_str());
                                        if ui.add(house::link(if armed { "Click again to restore" } else { "Restore" })).clicked() {
                                            if armed {
                                                restore = Some(id.clone());
                                            } else {
                                                self.backup_armed = Some(id.clone());
                                            }
                                        }
                                    });
                                });
                            }
                        });
                    }
                }
            }
            self.messages(ui);
            let can_run = set_up && enabled;
            let labels: &[&str] = if can_run {
                &["Close", "Back up now", "Save"]
            } else if set_up {
                &["Close", "Save"]
            } else {
                &["Cancel", "Set up backups"]
            };
            match house::actions(ui, labels) {
                Some(0) => cancel = true,
                Some(1) if can_run => now = true,
                Some(_) => save = true,
                None => {}
            }
        });
        if close || cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = Dialog::None;
            self.backup_key = None;
        } else if self.busy {
            // One request at a time.
        } else if let Some(enabled) = master {
            self.busy = true;
            self.error = None;
            call(&self.inbox, ctx, Method::Post, "api/backup", Some(json!({"enabled": enabled})), |v| {
                Reply::BackupEnabled(v["enabled"].as_bool() == Some(true))
            });
        } else if save {
            self.busy = true;
            self.error = None;
            let mut body = json!({ "to": self.backup_to.trim(), "on": self.backup_on, "hour": self.backup_hour });
            if self.backup_has_old && !self.backup_old_key.trim().is_empty() {
                body["key"] = json!(self.backup_old_key.trim());
            }
            call(&self.inbox, ctx, Method::Post, "api/backup", Some(body), |v| {
                Reply::BackupDone(match v["result"].as_str() {
                    Some("made") => "Backups are set up. Now keep a copy of the recovery key (below) somewhere else.".into(),
                    Some("found") => "Found the backups in that folder; they'll carry on there.".into(),
                    _ => "Saved.".into(),
                })
            });
        } else if now {
            self.busy = true;
            self.error = None;
            self.backup_polled = 0.0;
            call(&self.inbox, ctx, Method::Post, "api/backup/run", None, |_| Reply::BackupDone("Backup started.".into()));
        } else if let Some(id) = restore {
            self.busy = true;
            self.error = None;
            self.backup_armed = None;
            self.done = Some("Restoring…".into());
            let mut body = json!({ "id": id });
            if let Some((_, Some(w))) = whose.get(self.backup_whose) {
                body["whose"] = json!(w);
            }
            call(&self.inbox, ctx, Method::Post, "api/backup/restore", Some(body), |v| {
                Reply::BackupDone(format!(
                    "Copied {} files into the folder \"{}\".",
                    v["files"].as_u64().unwrap_or(0),
                    v["folder"].as_str().unwrap_or("")
                ))
            });
        } else if show_key {
            call(&self.inbox, ctx, Method::Get, "api/backup/key", None, |v| Reply::BackupKey(v["key"].as_str().unwrap_or("").to_string()));
        }
    }

    /// After a change of folder: "Starbase is restarting…" until the server answers again
    /// (checked once a second), then the page starts again.
    fn restarting_ui(&mut self, ctx: &egui::Context) {
        let Some(since) = self.restarting else { return };
        let now = ctx.input(|i| i.time);
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        if now - since > 1.5 && now - self.polled > 1.0 {
            self.polled = now;
            wasm_bindgen_futures::spawn_local(async {
                if matches!(fetch(Method::Get, "api/info", None, None).await, Ok((200, _, _))) {
                    reload();
                }
            });
        }
        house::dialog(ctx, "starsheets_restarting", 360.0, "Moving to the new folder", None, |ui| {
            house::text(ui, "Starbase is restarting in its new folder. This page starts again by itself in a few seconds.");
        });
    }

    /// The sign-in box is a real HTML form (index.html) so password managers can fill it; the
    /// form signs in itself and reloads the page. Here: show it, hide it, and follow its
    /// "Create an account" link.
    fn sign_in_dom(&mut self, ctx: &egui::Context) {
        let want = self.dialog == Dialog::SignIn;
        let Some(w) = web_sys::window() else { return };
        if want != self.dom_sign_in {
            self.dom_sign_in = want;
            if let Some(f) = js_sys::Reflect::get(&w, &"__ssSignIn".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) {
                let sso = self.sso.as_ref().map_or(wasm_bindgen::JsValue::NULL, |s| json!({ "label": s.label, "only": s.only }).to_string().into());
                let _ = f.call3(&w, &want.into(), &self.signup.into(), &sso);
            }
        }
        if want && js_sys::Reflect::get(&w, &"__ssWantSignUp".into()).is_ok_and(|v| v.is_truthy()) {
            let _ = js_sys::Reflect::set(&w, &"__ssWantSignUp".into(), &false.into());
            self.open(Dialog::SignUp);
            ctx.request_repaint();
        }
    }

    /// Making your own account (only offered when the owner allows sign-up).
    fn sign_up(&mut self, ctx: &egui::Context) {
        let mut go = false;
        let mut back = false;
        house::dialog(ctx, "starsheets_sign_up", 400.0, "Create an account", None, |ui| {
            house::label(ui, "Your name", "What other people see next to your cursor.");
            let r = Self::field(ui, &mut self.name, "Name", false);
            self.first(&r);
            ui.add_space(house::S);
            house::label(ui, "Username and password", "For signing in. The username is letters and numbers.");
            Self::field(ui, &mut self.username, "Username", false);
            ui.add_space(house::S);
            Self::field(ui, &mut self.password, "Password (8 characters or more)", true);
            ui.add_space(house::S);
            let r = Self::field(ui, &mut self.password2, "Password again", true);
            go |= Self::enter(ui, &r);
            if self.signup_code {
                house::gap(ui);
                house::label(ui, "Sign-up code", "The person who runs this server gives you this.");
                let r = Self::field(ui, &mut self.code, "Sign-up code", false);
                go |= Self::enter(ui, &r);
            }
            self.messages(ui);
            if ui.add(house::link("Already have an account? Sign in")).clicked() {
                back = true;
            }
            go |= house::actions(ui, &["Create account"]).is_some();
        });
        if back {
            self.open(Dialog::SignIn);
        } else if go && !self.busy {
            if self.password != self.password2 {
                self.error = Some("The two passwords are different.".into());
                return;
            }
            self.busy = true;
            self.error = None;
            let body = json!({ "code": self.code.trim(), "name": self.name, "username": self.username, "password": self.password });
            call(&self.inbox, ctx, Method::Post, "api/signup", Some(body), |_| Reply::SignedIn);
        }
    }

    fn setup(&mut self, ctx: &egui::Context) {
        let mut go = false;
        house::dialog(ctx, "starsheets_setup", 400.0, "Welcome to Starbase", None, |ui| {
            house::note(ui, "This server has no accounts yet. Make yours: you'll be its owner and can add other people afterwards.");
            house::note(
                ui,
                "The setup code is printed in the server's log when it starts (with Docker: docker logs followed by the container's name). If the server was started with STARBASE_PASSWORD (or the older STARSHEETS_PASSWORD), that is the code.",
            );
            if let Some(sso) = self.sso.as_ref().filter(|s| s.owner_group) {
                house::note(ui, "Or, if you are in the owner group at your sign-in service, sign in with it: you become the owner.");
                if ui.add(house::secondary(&sso.label)).clicked() {
                    go_sso_login();
                }
            }
            house::gap(ui);
            house::label(ui, "Setup code", "");
            let r = Self::field(ui, &mut self.code, "From the server's log", false);
            self.first(&r);
            // With the code typed, ask again for the folders files could be kept in.
            let code = self.code.trim().to_string();
            if code.len() >= 10 && code != self.storage_code {
                self.storage_code = code.clone();
                let url = format!("api/storage?code={}", crate::files::enc(&code));
                call(&self.inbox, ctx, Method::Get, &url, None, Reply::Storage);
            }
            house::gap(ui);
            house::label(ui, "Your name", "What other people see next to your cursor.");
            Self::field(ui, &mut self.name, "Name", false);
            ui.add_space(house::S);
            house::label(ui, "Username and password", "For signing in. The username is letters and numbers.");
            Self::field(ui, &mut self.username, "Username", false);
            ui.add_space(house::S);
            Self::field(ui, &mut self.password, "Password (8 characters or more)", true);
            ui.add_space(house::S);
            let r = Self::field(ui, &mut self.password2, "Password again", true);
            go |= Self::enter(ui, &r);
            // Where files go: the server's own folder, or a folder inside it.
            let folders: Vec<String> = self
                .storage
                .as_ref()
                .map(|v| v["folders"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if !folders.is_empty() {
                house::gap(ui);
                house::label(
                    ui,
                    "Where to keep files",
                    "Your spreadsheets are saved here. You can change it later (your avatar › Where files are kept).",
                );
                let mut labels = vec!["The top folder".to_string()];
                labels.extend(folders.iter().map(|f| f.replace('/', " › ")));
                let mut i = self.storage_pick.as_ref().and_then(|p| folders.iter().position(|f| f == p)).map_or(0, |i| i + 1);
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                if house::select(ui, "starsheets_setup_folder", ui.available_width(), &mut i, &refs, &[0]) {
                    self.storage_pick = Some(if i == 0 { String::new() } else { folders[i - 1].clone() });
                }
            }
            self.messages(ui);
            go |= house::actions(ui, &["Create account"]).is_some();
        });
        if go && !self.busy {
            if self.password != self.password2 {
                self.error = Some("The two passwords are different.".into());
                return;
            }
            self.busy = true;
            self.error = None;
            let body = json!({ "code": self.code.trim(), "name": self.name, "username": self.username, "password": self.password });
            call(&self.inbox, ctx, Method::Post, "api/setup", Some(body), |_| Reply::SetUp);
        }
    }

    fn profile(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut cancel = false;
        let mut go = false;
        house::dialog(ctx, "starsheets_profile", 380.0, "Your name and colour", Some(&mut close), |ui| {
            house::label(ui, "Name", "Shown next to your cursor and on your avatar.");
            let r = Self::field(ui, &mut self.name, "Name", false);
            self.first(&r);
            go |= Self::enter(ui, &r);
            house::gap(ui);
            house::label(ui, "Colour", "");
            ui.horizontal_wrapped(|ui| {
                for c in COLOURS {
                    let picked = self.colour.eq_ignore_ascii_case(c);
                    let (rect, r) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                    ui.painter().circle_filled(rect.center(), 12.0, parse_colour(c));
                    if picked {
                        ui.painter().circle_stroke(rect.center(), 13.5, egui::Stroke::new(2.0, theme::text()));
                    }
                    if r.clicked() {
                        self.colour = c.to_string();
                    }
                }
            });
            if let Some(sso) = self.sso.clone() {
                house::gap(ui);
                if sso.linked && sso.password {
                    house::label(ui, "Single sign-on", "Your account is linked to your sign-in at the sign-in service.");
                    if ui.add(house::link("Unlink")).clicked() {
                        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(REQUEST), "sso-unlink".to_string()));
                    }
                } else if sso.linked {
                    // Made through single sign-on: no password, so it can't simply be unlinked.
                    // Setting a password or linking a different sign-in needs a fresh sign-in
                    // there (the server checks: api/oidc/login?fresh=1, then 10 minutes).
                    house::label(
                        ui,
                        "Single sign-on",
                        "Your account was made through the sign-in service and has no password here, so you sign in only through it.",
                    );
                    if sso.fresh {
                        house::note(
                            ui,
                            "You've just confirmed who you are. For the next 10 minutes you can set a password (then you can unlink), or link a different sign-in to this account.",
                        );
                        ui.add_space(house::S);
                        let mut link_other = false;
                        ui.horizontal(|ui| {
                            if ui.add(house::secondary("Set a password…")).clicked() {
                                ctx.data_mut(|m| m.insert_temp(egui::Id::unique(REQUEST), "password".to_string()));
                            }
                            link_other = ui.add(house::secondary("Link another sign-in")).clicked();
                        });
                        if link_other && !self.busy {
                            self.busy = true;
                            call(&self.inbox, ctx, Method::Post, "api/oidc/link", Some(json!({ "next": here() })), |v| match v["url"].as_str() {
                                Some(u) => Reply::Go(u.to_string()),
                                None => Reply::Error("The sign-in service couldn't be reached.".into()),
                            });
                        }
                    } else {
                        house::note(
                            ui,
                            "To set a password (then you can unlink) or link a different sign-in, first confirm who you are at the sign-in service.",
                        );
                        ui.add_space(house::S);
                        if ui.add(house::secondary("Confirm who you are")).clicked() {
                            go_fresh_login();
                        }
                    }
                } else {
                    house::label(ui, "Single sign-on", "Link this account to your sign-in at the sign-in service, then use it to sign in here.");
                    if sso.password {
                        Self::field(ui, &mut self.current, "Your password here (to link)", true);
                        ui.add_space(house::S);
                    }
                    if ui.add(house::secondary(&sso.label)).clicked() && !self.busy {
                        self.busy = true;
                        let body = json!({ "current_password": self.current, "next": here() });
                        call(&self.inbox, ctx, Method::Post, "api/oidc/link", Some(body), |v| match v["url"].as_str() {
                            Some(u) => Reply::Go(u.to_string()),
                            None => Reply::Error("The sign-in service couldn't be reached.".into()),
                        });
                    }
                }
            }
            self.messages(ui);
            match house::actions(ui, &["Cancel", "Save"]) {
                Some(0) => cancel = true,
                Some(_) => go = true,
                None => {}
            }
        });
        if close || cancel {
            self.dialog = Dialog::None;
        } else if go && !self.busy {
            self.busy = true;
            call(&self.inbox, ctx, Method::Post, "api/me", Some(json!({ "name": self.name, "colour": self.colour })), |v| {
                match me_from(&v["user"]) {
                    Some(me) => Reply::Saved(me),
                    None => Reply::Error("Couldn't save.".into()),
                }
            });
        }
    }

    fn change_password(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut cancel = false;
        let mut go = false;
        // An account made through single sign-on sets its first password: there is no current
        // one to type (the server wants a fresh sign-in at the provider instead).
        let first = !HAS_PASSWORD.with(|p| *p.borrow());
        let title = if first { "Set a password" } else { "Change password" };
        house::dialog(ctx, "starsheets_password", 380.0, title, Some(&mut close), |ui| {
            if !first {
                let r = Self::field(ui, &mut self.current, "Current password", true);
                self.first(&r);
                ui.add_space(house::S);
            }
            let r = Self::field(ui, &mut self.password, "New password (8 characters or more)", true);
            if first {
                self.first(&r);
            }
            ui.add_space(house::S);
            let r = Self::field(ui, &mut self.password2, "New password again", true);
            go |= Self::enter(ui, &r);
            house::note(
                ui,
                if first {
                    "Then you can also sign in here with your username and this password, and unlink single sign-on if you want to."
                } else {
                    "You'll stay signed in here; other devices will need the new password."
                },
            );
            self.messages(ui);
            match house::actions(ui, &["Cancel", title]) {
                Some(0) => cancel = true,
                Some(_) => go = true,
                None => {}
            }
        });
        if close || cancel {
            self.dialog = Dialog::None;
        } else if go && !self.busy {
            if self.password != self.password2 {
                self.error = Some("The two new passwords are different.".into());
                return;
            }
            self.busy = true;
            let body =
                if first { json!({ "password": self.password }) } else { json!({ "current_password": self.current, "password": self.password }) };
            call(&self.inbox, ctx, Method::Post, "api/me", Some(body), move |v| match me_from(&v["user"]) {
                // The first password: the page starts again, now knowing the account has one.
                Some(_) if first => Reply::Reload,
                Some(me) => Reply::Saved(me),
                None => Reply::Error("Couldn't change the password.".into()),
            });
        }
    }

    fn load_devices(&mut self, ctx: &egui::Context) {
        self.devices = None;
        let url = if self.devices_of.is_empty() {
            "api/devices".to_string()
        } else {
            format!("api/devices?username={}", crate::files::enc(&self.devices_of))
        };
        call(&self.inbox, ctx, Method::Get, &url, None, |v| Reply::Devices(v.as_array().cloned().unwrap_or_default()));
    }

    /// Your devices: the desktop apps connected to your account ("Connect to a server…" in
    /// Starbase Sheets and Docs), when each was last used, and Remove (it stops working at once).
    fn devices_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut done = false;
        let mut remove = None;
        let mine = self.devices_of.is_empty();
        let title = if mine { "Your devices".to_string() } else { format!("{}'s devices", self.devices_of) };
        let now = js_sys::Date::now() / 1000.0;
        house::dialog(ctx, "starsheets_devices", 440.0, &title, Some(&mut close), |ui| {
            house::note(
                ui,
                if mine {
                    "Desktop apps connected to your account. Remove one you no longer use, or don't recognise: it stops working at once."
                } else {
                    "Desktop apps connected to this account. Removing one stops it working at once."
                },
            );
            house::gap(ui);
            match &self.devices {
                None => house::note(ui, "Loading…"),
                Some(list) if list.is_empty() => {
                    house::note(ui, "None. Connect one from Starbase Sheets or Docs on your computer: Connect to a server…")
                }
                Some(list) => {
                    for d in list.clone() {
                        let id = d["id"].as_str().unwrap_or("").to_string();
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let name = d["name"].as_str().unwrap_or("A desktop app");
                                ui.label(RichText::new(name).size(house::BODY).color(theme::text()));
                                let used = d["used"].as_f64().map_or_else(String::new, |t| ago(now - t));
                                let this = if d["current"].as_bool() == Some(true) { " · this app" } else { "" };
                                house::note(ui, &format!("Last used {used}{this}"));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let armed = self.device_armed.as_deref() == Some(id.as_str());
                                if ui.add(house::link(if armed { "Click again to remove" } else { "Remove" })).clicked() {
                                    if armed {
                                        remove = Some((id.clone(), d["name"].as_str().unwrap_or("").to_string()));
                                    } else {
                                        self.device_armed = Some(id.clone());
                                    }
                                }
                            });
                        });
                        ui.add_space(house::XS);
                    }
                }
            }
            self.messages(ui);
            if house::actions(ui, &["Close"]).is_some() {
                done = true;
            }
        });
        if close || done {
            self.dialog = Dialog::None;
        } else if let Some((id, name)) = remove {
            self.device_armed = None;
            let mut url = format!("api/devices?id={}", crate::files::enc(&id));
            if !mine {
                url.push_str(&format!("&username={}", crate::files::enc(&self.devices_of)));
            }
            call(&self.inbox, ctx, Method::Delete, &url, None, move |_| Reply::DeviceGone(format!("Removed {name}.")));
        }
    }

    fn people(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut cancel = false;
        let mut add = false;
        let mut remove = None;
        let mut unlink: Option<String> = None;
        let mut save_signup = false;
        let mut save_preview = false;
        let mut devices_of = None;
        let me = me().unwrap_or_default();
        house::dialog(ctx, "starsheets_people", 460.0, "People", Some(&mut close), |ui| {
            house::note(ui, "Everyone with an account on this server. Each person sees only their own files and the ones shared with them.");
            house::gap(ui);
            for Person { me: p, link: linked, password } in self.people.clone() {
                ui.horizontal(|ui| {
                    avatar(ui, &p.name, &p.colour, 28.0);
                    ui.add_space(house::S);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&p.name).size(house::BODY).color(theme::text()));
                        ui.horizontal(|ui| {
                            house::note(ui, &if p.owner { format!("{} · owner", p.username) } else { p.username.clone() });
                            if ui.add(house::link("Devices")).on_hover_text("The desktop apps connected to this account").clicked() {
                                devices_of = Some(p.username.clone());
                            }
                        });
                        if let Some((shown, ids)) = &linked {
                            ui.horizontal(|ui| {
                                house::note_with_tip(ui, &format!("Single sign-on: {shown}"), ids);
                                // No password: removing the link would lock them out (the
                                // server refuses); a password set here removes it instead.
                                if password {
                                    if ui.add(house::link("Remove link")).clicked() {
                                        unlink = Some(p.username.clone());
                                    }
                                } else {
                                    house::note_with_tip(
                                        ui,
                                        "· no password",
                                        "They sign in only through single sign-on, so the link stays until they set a password themselves (Your name and colour).",
                                    );
                                }
                            });
                        }
                    });
                    if !p.owner && p.username != me.username {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let armed = self.remove_armed.as_deref() == Some(p.username.as_str());
                            if ui.add(house::link(if armed { "Click again to remove" } else { "Remove" })).clicked() {
                                if armed {
                                    remove = Some(p.username.clone());
                                } else {
                                    self.remove_armed = Some(p.username.clone());
                                }
                            }
                        });
                    }
                });
                ui.add_space(house::XS);
            }
            house::gap(ui);
            house::label(
                ui,
                "Sign-up",
                "Let people make their own account from the sign-in box. Only turn this on if strangers can't reach your server, or set a code.",
            );
            let mut changed = ui.checkbox(&mut self.signup_on, "Let people make their own account").changed();
            if self.signup_on {
                ui.add_space(house::S);
                let r = Self::field(ui, &mut self.signup_secret, "Sign-up code (optional, you give it out)", false);
                changed |= r.lost_focus();
            }
            if changed {
                save_signup = true;
            }
            house::gap(ui);
            house::label(
                ui,
                "Link previews",
                "When someone rests the pointer on a link in a sheet, the server fetches that page's title and picture to show on a card. Off by default; web addresses on your own network are never fetched.",
            );
            let r = ui.checkbox(&mut self.link_preview_on, "Show a page preview when hovering a link");
            house::target(ui.ctx(), "people:link-preview", r.rect);
            LINK_PREVIEW_SHOWN.with(|c| c.set(Some(self.link_preview_on)));
            if r.changed() {
                save_preview = true;
            }
            house::gap(ui);
            house::label(ui, "Add someone", "Give them their username and this password; they can change the password after signing in.");
            Self::field(ui, &mut self.name, "Their name", false);
            ui.add_space(house::S);
            Self::field(ui, &mut self.username, "Username", false);
            ui.add_space(house::S);
            Self::field(ui, &mut self.password, "Password for them (8 characters or more)", false);
            self.messages(ui);
            match house::actions(ui, &["Close", "Add"]) {
                Some(0) => cancel = true,
                Some(_) => add = true,
                None => {}
            }
        });
        if let Some(who) = devices_of {
            self.open(Dialog::Devices);
            self.devices_of = who;
            self.load_devices(ctx);
        } else if close || cancel {
            self.dialog = Dialog::None;
        } else if add && !self.busy {
            self.busy = true;
            let who = self.username.clone();
            call(
                &self.inbox,
                ctx,
                Method::Post,
                "api/users",
                Some(json!({ "name": self.name, "username": self.username, "password": self.password })),
                move |_| Reply::Done(format!("Added {who}.")),
            );
        } else if save_signup {
            let body = json!({ "on": self.signup_on, "code": self.signup_secret.trim() });
            let on = self.signup_on;
            call(&self.inbox, ctx, Method::Post, "api/signup/settings", Some(body), move |_| {
                Reply::Done(if on { "Sign-up is on.".into() } else { "Sign-up is off.".into() })
            });
        } else if save_preview {
            let on = self.link_preview_on;
            call(&self.inbox, ctx, Method::Post, "api/link-preview/settings", Some(json!({ "on": on })), move |_| {
                Reply::Done(if on { "Link previews are on.".into() } else { "Link previews are off.".into() })
            });
        } else if let Some(who) = remove {
            self.remove_armed = None;
            let url = format!("api/users?username={}", crate::files::enc(&who));
            call(&self.inbox, ctx, Method::Delete, &url, None, move |_| Reply::Done(format!("Removed {who}.")));
        } else if let Some(who) = unlink {
            let url = format!("api/users/sso?username={}", crate::files::enc(&who));
            call(&self.inbox, ctx, Method::Delete, &url, None, move |_| Reply::Done(format!("Removed {who}'s single sign-on link.")));
        }
    }
}
