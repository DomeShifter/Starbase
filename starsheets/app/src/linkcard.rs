//! Sheets' link card. Resting the pointer on a cell with a link (a link set with Insert › Link, a
//! `=HYPERLINK(…)` formula, or a cell whose text is a web or e-mail address) shows a small card
//! after a short wait, as Google Sheets does; selecting a linked cell shows it too (patch 0009
//! stops GridCraft's click-to-follow). The card shows the site's name and the address, with Open,
//! Copy link and, where the user may edit, Edit link and Remove link.
//!
//! **Page preview: owner opt-in, off by default** (docs/privacy.md). When the owner turned it on
//! (People › Link previews), resting the pointer on a web link asks our own server
//! (`api/link-preview`) for the page title, site icon and a small picture; the browser never
//! contacts the other site. Off, offline or failed: the card stays as before (globe + address).

use egui::{Align2, Button, Pos2, Rect, RichText, Ui, vec2};
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::links::{LinkKind, classify};
use gridcraft_engine::model::Sheet;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, icons, theme};

/// How long the pointer rests on a linked cell before the card shows (seconds).
const HOVER_DELAY: f64 = 0.5;
/// The longest address shown in full; a longer one keeps its start and end with "…" between.
const URL_SHOWN_MAX: usize = 52;

/// Where a cell's link comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Stored on the cell (Insert › Link, a link from a file, or a typed address turned into one).
    Stored,
    /// The cell's `=HYPERLINK(…)` formula.
    Formula,
    /// The cell's text is itself a web or e-mail address.
    Text,
}

/// A cell's link, as the card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub source: Source,
}

impl Link {
    /// What the card does with it (the engine's decision: http, https and mailto only open).
    pub fn kind(&self) -> LinkKind {
        classify(&self.target)
    }
}

/// The link in `cell`, if it has one.
pub fn link_of(sh: &Sheet, cell: CellRef) -> Option<Link> {
    if let Some(h) = sh.hyperlinks.get(&cell) {
        return Some(Link { target: h.target.clone(), source: Source::Stored });
    }
    if let Some(f) = sh.cell(cell).and_then(|c| c.formula.as_ref())
        && let Some(target) = hyperlink_formula_target(sh, &f.text)
    {
        return Some(Link { target, source: Source::Formula });
    }
    let Value::Text(t) = sh.value(cell) else { return None };
    let t = t.to_string();
    let t = t.trim();
    let lower = t.to_ascii_lowercase();
    let address = ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p) && lower.len() > p.len());
    (address && !t.contains(char::is_whitespace)).then(|| Link { target: t.to_string(), source: Source::Text })
}

/// The address in a `=HYPERLINK("…", …)` formula: a text in quotes, or one cell on this sheet.
/// Anything worked out (`"https://" & A1`) isn't read here; the card still shows when the result is an address.
fn hyperlink_formula_target(sh: &Sheet, formula: &str) -> Option<String> {
    let f = formula.trim().trim_start_matches('=').trim_start();
    let head = f.get(..10)?;
    if !head.eq_ignore_ascii_case("HYPERLINK(") {
        return None;
    }
    let rest = f[10..].trim_start();
    if let Some(body) = rest.strip_prefix('"') {
        // A text in quotes; "" inside is one quotation mark.
        let mut out = String::new();
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    out.push('"');
                } else {
                    let after = chars.collect::<String>();
                    let after = after.trim_start();
                    return (after.starts_with(',') || after.starts_with(')')).then_some(out).filter(|s| !s.trim().is_empty());
                }
            } else {
                out.push(c);
            }
        }
        return None;
    }
    let end = rest.find([',', ')'])?;
    let at = CellRef::parse(&rest[..end].trim().replace('$', ""))?;
    match sh.value(at) {
        Value::Text(t) => Some(t.to_string()).filter(|s| !s.trim().is_empty()),
        _ => None,
    }
}

/// The card's heading: the site's name (`example.com`), the e-mail address, or what kind of link
/// it is. Any `user:password@` before a web address's host is dropped, so
/// `https://bank.example@evil.example` reads `evil.example`.
pub fn heading(kind: &LinkKind) -> String {
    match kind {
        LinkKind::External(url) => {
            let lower = url.to_ascii_lowercase();
            if lower.starts_with("mailto:") {
                return url[7..].split('?').next().unwrap_or_default().to_string();
            }
            let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
            let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let host = authority.rsplit('@').next().unwrap_or_default();
            let host = if host.starts_with('[') { host.split_inclusive(']').next().unwrap_or(host) } else { host.split(':').next().unwrap_or(host) };
            let host = host.to_ascii_lowercase();
            host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
        }
        LinkKind::Internal(_) => "In this spreadsheet".into(),
        LinkKind::Refused(_) => "This link isn't opened".into(),
    }
}

/// `url` shortened in the middle when it's long, so both the site and the page stay readable.
pub fn middle_truncate(url: &str, max: usize) -> String {
    let n = url.chars().count();
    if n <= max {
        return url.to_string();
    }
    let tail = (max - 1) / 2;
    let head = max - 1 - tail;
    let start: String = url.chars().take(head).collect();
    let end: String = url.chars().skip(n - tail).collect();
    format!("{start}…{end}")
}

/// What the card currently shows, for the test hook (`linkCard` in the snapshot).
#[cfg(target_arch = "wasm32")]
pub fn shown() -> serde_json::Value {
    SHOWN.with(|s| s.borrow().clone())
}

thread_local! {
    static SHOWN: std::cell::RefCell<serde_json::Value> = const { std::cell::RefCell::new(serde_json::Value::Null) };
}

/// The pointer resting on a linked cell, since when (egui time).
#[derive(Clone, Copy)]
struct Rest {
    cell: CellRef,
    since: f64,
}

#[derive(Default)]
pub struct LinkCard {
    /// The selected cell's card: where it was drawn (a fallback while the cell is measured).
    at: Option<(CellRef, Pos2)>,
    /// Closed with ✕ or Esc for this cell.
    dismissed: Option<CellRef>,
    /// A web or e-mail link waiting for "Open" (GridCraft's Open Hyperlink box, in the house
    /// style: to-do row 63).
    confirm: Option<String>,
    /// The pointer resting on a linked cell, waiting for the delay.
    rest: Option<Rest>,
    /// The hovered cell whose card is showing.
    hovered: Option<CellRef>,
    /// The card's rectangle last frame: the pointer may move onto it without the card closing.
    card_rect: Option<Rect>,
    /// The grid's scroll last frame: scrolling closes a hover card.
    scroll: Option<egui::Vec2>,
}

impl LinkCard {
    /// Takes GridCraft's Open Hyperlink box (runs before and after the grid; draws nothing).
    pub fn intercept(&mut self, app: &mut SheetApp) {
        if let Some(d) = app.dialog.as_ref().filter(|d| d.name == "openLink") {
            self.confirm = d.values.get("url").and_then(serde_json::Value::as_str).map(str::to_string);
            app.dialog = None;
        }
    }

    /// "Open this link?" in the house style; Open hands the address to GridCraft, which opens
    /// only web and e-mail addresses.
    pub fn confirm_ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let Some(url) = self.confirm.clone() else { return };
        let mut closed = false;
        let hit = house::dialog(ctx, "starsheets_open_link", 420.0, "Open this link?", Some(&mut closed), |ui| {
            house::note(ui, "It opens in a new tab. Only open links you trust: a link can lead to a harmful site.");
            ui.add_space(house::S);
            ui.add(egui::Label::new(RichText::new(&url).size(house::BODY).color(theme::BLUE)).wrap());
            house::actions(ui, &["Cancel", "Open"])
        });
        match hit {
            Some(1) => {
                open_now(app, &url);
                self.confirm = None;
            }
            Some(_) => self.confirm = None,
            None if closed || ctx.input(|i| i.key_pressed(egui::Key::Escape)) => self.confirm = None,
            None => {}
        }
    }

    /// The pointer's cell, if it rests on a linked one; keeps the hover card while the pointer is
    /// on the cell or the card. Returns the cell whose hover card should show.
    fn hover(&mut self, app: &SheetApp, ctx: &egui::Context) -> Option<CellRef> {
        let scroll = app.view().scroll;
        let scrolled = self.scroll.is_some_and(|s| s != scroll) || ctx.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO);
        self.scroll = Some(scroll);
        let (pointer, now, busy) = ctx.input(|i| (i.pointer.latest_pos(), i.time, i.pointer.any_down()));
        if scrolled || busy || ctx.dragged_id().is_some() || app.dialog.is_some() || self.confirm.is_some() {
            self.rest = None;
            self.hovered = None;
            return None;
        }
        let pointer = pointer?;
        // Still on the card (or the gap above it): keep it.
        if let (Some(c), Some(r)) = (self.hovered, self.card_rect)
            && r.expand2(vec2(0.0, 8.0)).contains(pointer)
        {
            return Some(c);
        }
        let sh = app.session.active()?.wb.active()?;
        // GridCraft's hover cell, checked against the pointer: it isn't cleared when the pointer
        // leaves the grid.
        let cell = app
            .grid
            .hover_cell
            .filter(|c| cell_rect(app, *c).is_some_and(|r| r.contains(pointer)))
            .filter(|_| app.grid.cells_rect.is_some_and(|r| r.contains(pointer)))
            .filter(|c| link_of(sh, *c).is_some());
        let Some(cell) = cell else {
            self.rest = None;
            self.hovered = None;
            return None;
        };
        if self.hovered == Some(cell) {
            return Some(cell);
        }
        self.hovered = None;
        let rest = *self.rest.get_or_insert(Rest { cell, since: now });
        let rest = if rest.cell == cell { rest } else { *self.rest.insert(Rest { cell, since: now }) };
        let waited = now - rest.since;
        if waited >= HOVER_DELAY {
            self.hovered = Some(cell);
            Some(cell)
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY - waited));
            None
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        SHOWN.with(|s| *s.borrow_mut() = serde_json::Value::Null);
        if app.editor.is_some() {
            self.rest = None;
            self.hovered = None;
            self.card_rect = None;
            return;
        }
        let hovered = self.hover(app, &ctx);
        let selected = app.session.active().and_then(|d| {
            let c = d.selection.active;
            let sh = d.wb.active()?;
            (d.selection.is_single_cell() && link_of(sh, c).is_some()).then_some(c)
        });
        if selected.is_none() {
            self.at = None;
        }
        // One card at a time: the hovered cell's wins over the selected cell's.
        let Some((cell, via_hover)) = hovered.map(|c| (c, true)).or(selected.map(|c| (c, false))) else {
            self.dismissed = None;
            self.card_rect = None;
            return;
        };
        if self.dismissed.is_some_and(|d| d != cell) {
            self.dismissed = None;
        }
        if self.dismissed == Some(cell) {
            self.card_rect = None;
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dismissed = Some(cell);
            self.hovered = None;
            self.card_rect = None;
            return;
        }
        let Some(link) = app.session.active().and_then(|d| d.wb.active()).and_then(|sh| link_of(sh, cell)) else { return };
        // Just under the cell's bottom-left corner, as in Sheets (re-measured every frame so it
        // follows scrolling); the pointer is only a fallback.
        let pos = cell_bottom_left(app, cell).map(|p| p + vec2(0.0, 4.0)).unwrap_or_else(|| {
            let p = self.at.filter(|(c, _)| *c == cell).map(|(_, p)| p);
            p.unwrap_or_else(|| ctx.input(|i| i.pointer.latest_pos()).unwrap_or_else(|| ui.max_rect().center()) + vec2(-12.0, 18.0))
        });
        if !via_hover {
            self.at = Some((cell, pos));
        }
        // Hidden while the cell is scrolled out of the grid.
        if app.grid.cells_rect.is_some_and(|r| !r.contains(pos - vec2(0.0, 6.0))) {
            self.card_rect = None;
            return;
        }
        let kind = link.kind();
        let can_edit = link.source == Source::Stored && !app.session.read_only;
        let shown_url = middle_truncate(&link.target, URL_SHOWN_MAX);
        let in_full = shown_url == link.target;
        let heading = heading(&kind);
        let can_open = !matches!(kind, LinkKind::Refused(_));
        // A page preview only for a web link the pointer rests on (never ahead of time).
        let web = matches!(&kind, LinkKind::External(u) if u.starts_with("http://") || u.starts_with("https://"));
        let preview = (via_hover && web).then(|| preview_for(&link.target, &ctx));
        let fade = preview.as_ref().filter(|p| p.state == "ok").map_or(0.0, |p| ctx.animate_bool(egui::Id::new(("link-preview-fade", &link.target)), !p.title.is_empty() || p.picture.is_some()));
        SHOWN.with(|s| {
            *s.borrow_mut() = json!({
                "cell": cell.a1(),
                "via": if via_hover { "hover" } else { "selection" },
                "url": link.target,
                "heading": heading,
                "shownUrl": shown_url,
                "open": can_open,
                "edit": can_edit,
                "source": format!("{:?}", link.source),
                "preview": preview.as_ref().map(|p| json!({
                    "state": p.state,
                    "title": p.title,
                    "site": p.site,
                    "icon": p.icon.is_some(),
                    "picture": p.picture.is_some(),
                })),
            })
        });
        let mut action = None;
        let area = egui::Area::new(egui::Id::unique("starsheets_link_card"))
            .fixed_pos(pos)
            .pivot(Align2::LEFT_TOP)
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                house::card_frame().inner_margin(egui::Margin::same(house::M as i8)).show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.spacing_mut().item_spacing = vec2(house::S, house::XS);
                    let ok = preview.as_ref().filter(|p| p.state == "ok");
                    ui.horizontal(|ui| {
                        // The site's own icon when the preview brought one, else the globe.
                        match ok.and_then(|p| p.icon.as_ref()) {
                            Some(tex) => {
                                let r = ui.add(egui::Image::new((tex.id(), vec2(20.0, 20.0))));
                                house::target(ui.ctx(), "linkcard:icon", r.rect);
                            }
                            None => {
                                if let Some(globe) = icons::icon("language") {
                                    ui.add(globe);
                                }
                            }
                        }
                        ui.vertical(|ui| {
                            let heading = ok.map(|p| p.site.as_str()).filter(|s| !s.is_empty()).unwrap_or(&heading);
                            ui.add(egui::Label::new(RichText::new(heading).size(house::BODY).strong().color(theme::TEXT)).truncate());
                            let url = ui.add(egui::Label::new(RichText::new(&shown_url).size(house::NOTE).color(theme::TEXT_DIM)).truncate());
                            if !in_full {
                                url.on_hover_text(&link.target);
                            }
                        });
                    });
                    // The page preview (owner opt-in, fetched by our server): title, then a small
                    // picture, fading in when they arrive; otherwise today's card unchanged.
                    if let Some(p) = ok.filter(|_| fade > 0.0) {
                        ui.scope(|ui| {
                            ui.multiply_opacity(fade);
                            if !p.title.is_empty() {
                                ui.add_space(house::XS);
                                let r = ui.add(egui::Label::new(RichText::new(&p.title).size(house::BODY).color(theme::TEXT)).wrap());
                                house::target(ui.ctx(), "linkcard:title", r.rect);
                            }
                            if let Some(tex) = &p.picture {
                                ui.add_space(house::XS);
                                let size = tex.size_vec2();
                                let scale = (348.0 / size.x).min(180.0 / size.y).min(1.0);
                                let r = ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(4.0));
                                house::target(ui.ctx(), "linkcard:picture", r.rect);
                            }
                        });
                    }
                    if let LinkKind::Refused(why) = &kind {
                        ui.add_space(house::XS);
                        ui.add(egui::Label::new(RichText::new(why).size(house::NOTE).color(theme::TEXT_DIM)).wrap());
                    }
                    ui.add_space(house::XS);
                    ui.horizontal(|ui| {
                        let button = |ui: &mut Ui, icon: &str, tip: &str| match icons::icon(icon) {
                            Some(img) => ui.add(Button::image(img).frame(false)).on_hover_text(tip),
                            None => ui.add(Button::new(tip).frame(false)),
                        };
                        if can_open {
                            let label = if matches!(kind, LinkKind::Internal(_)) { "Go to" } else { "Open" };
                            let open = match icons::icon("open_in_new") {
                                Some(img) => Button::image_and_text(img, RichText::new(label).size(house::BODY).color(house::BLUE)),
                                None => Button::new(RichText::new(label).size(house::BODY).color(house::BLUE)),
                            };
                            if ui
                                .add(open.frame(false))
                                .on_hover_text(if label == "Open" { "Open in a new tab" } else { "Go to this place" })
                                .clicked()
                            {
                                action = Some(Action::Open);
                            }
                        }
                        if button(ui, "content_copy", "Copy link").clicked() {
                            action = Some(Action::Copy);
                        }
                        if can_edit {
                            if button(ui, "edit", "Edit link").clicked() {
                                action = Some(Action::Edit);
                            }
                            if button(ui, "link_off", "Remove link").clicked() {
                                action = Some(Action::Remove);
                            }
                        }
                        if !via_hover && ui.add(Button::new("✕").frame(false)).on_hover_text("Close").clicked() {
                            action = Some(Action::Close);
                        }
                    });
                });
            });
        self.card_rect = Some(area.response.rect);
        let Some(action) = action else { return };
        // Edit and Remove act on the selection: a hovered card's cell is selected first.
        if via_hover && matches!(action, Action::Edit | Action::Remove) {
            let _ = app.session.run("selection.set", json!({ "cell": cell.a1() }));
        }
        match action {
            Action::Open => match kind {
                // The card shows the whole address: that's the confirmation. A shortened one
                // asks first, in a box that shows all of it.
                LinkKind::External(url) if in_full => open_now(app, &url),
                LinkKind::External(url) => self.confirm = Some(url),
                _ => gridcraft_ui_egui::grid::follow_link(app, &link.target),
            },
            Action::Copy => ctx.copy_text(link.target.clone()),
            Action::Edit => app.open_dialog("insertLink", json!({})),
            Action::Remove => app.run_or_alert("edit.clearHyperlinks", json!({})),
            Action::Close => self.dismissed = Some(cell),
        }
        if via_hover && !matches!(action, Action::Copy) {
            self.hovered = None;
            self.rest = None;
            self.dismissed = Some(cell);
        }
    }
}

/// A web page's preview as the server sent it (`GET api/link-preview`). The browser never asks
/// the other site: the server fetches (only when its owner turned previews on).
#[derive(Clone, Default)]
pub struct Preview {
    /// "loading", "ok", "none", "off", "busy" or "error".
    pub state: String,
    pub title: String,
    pub site: String,
    pub icon: Option<egui::TextureHandle>,
    pub picture: Option<egui::TextureHandle>,
}

thread_local! {
    static PREVIEWS: std::cell::RefCell<std::collections::HashMap<String, Preview>> = Default::default();
}

/// The preview for `url`, asking the server the first time (only called once the pointer rested).
fn preview_for(url: &str, ctx: &egui::Context) -> Preview {
    if let Some(p) = PREVIEWS.with(|m| m.borrow().get(url).cloned()) {
        return p;
    }
    PREVIEWS.with(|m| m.borrow_mut().insert(url.to_string(), Preview { state: "loading".into(), ..Default::default() }));
    #[cfg(target_arch = "wasm32")]
    {
        let (url, ctx) = (url.to_string(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let p = fetch_preview(&url, &ctx).await;
            PREVIEWS.with(|m| m.borrow_mut().insert(url, p));
            ctx.request_repaint();
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The desktop app has no server to ask: today's card.
        let _ = ctx;
        PREVIEWS.with(|m| m.borrow_mut().insert(url.to_string(), Preview { state: "off".into(), ..Default::default() }));
    }
    PREVIEWS.with(|m| m.borrow().get(url).cloned()).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
async fn fetch_preview(url: &str, ctx: &egui::Context) -> Preview {
    use crate::files::{Method, enc, fetch};
    let q = enc(url);
    let error = Preview { state: "error".into(), ..Default::default() };
    let Ok((200, _, bytes)) = fetch(Method::Get, &format!("api/link-preview?url={q}"), None, None).await else { return error };
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let text = |k: &str| v[k].as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(120).collect::<String>();
    let mut p = Preview { state: v["state"].as_str().unwrap_or("error").to_string(), title: text("title"), site: text("site"), icon: None, picture: None };
    for kind in ["icon", "picture"] {
        if v[kind].as_bool() != Some(true) {
            continue;
        }
        let Ok((200, _, png)) = fetch(Method::Get, &format!("api/link-preview/image?url={q}&kind={kind}"), None, None).await else { continue };
        let Ok(img) = image::load_from_memory_with_format(&png, image::ImageFormat::Png) else { continue };
        let rgba = img.into_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
        let tex = ctx.load_texture(format!("link-preview-{kind}-{url}"), ci, egui::TextureOptions::LINEAR);
        if kind == "icon" { p.icon = Some(tex) } else { p.picture = Some(tex) }
    }
    p
}

#[derive(Clone, Copy)]
enum Action {
    Open,
    Copy,
    Edit,
    Remove,
    Close,
}

/// Opens an address the user has seen in full and chosen to open (a new tab, `noopener`).
fn open_now(app: &mut SheetApp, url: &str) {
    if let Err(e) = app.session.open_confirmed_link(url) {
        app.message = Some(("Open this link?".into(), e.to_string()));
    }
    app.after_engine();
}

/// The on-screen bottom-left corner of `cell` on the active sheet, from GridCraft's own geometry.
pub(crate) fn cell_bottom_left(app: &SheetApp, cell: CellRef) -> Option<egui::Pos2> {
    cell_rect(app, cell).map(|r| r.left_bottom())
}

/// Where a cell is on screen.
pub(crate) fn cell_rect(app: &SheetApp, cell: CellRef) -> Option<egui::Rect> {
    let cells = app.grid.cells_rect?;
    let sh = app.session.active()?.wb.active()?;
    let scroll = app.view().scroll;
    // Geo takes the whole grid area (headings included); find the headings' size from a probe.
    let probe = gridcraft_ui_egui::grid::Geo::new(sh, cells, scroll);
    let grid = cells.translate(cells.min - probe.cells.min);
    let geo = gridcraft_ui_egui::grid::Geo::new(sh, grid, scroll);
    Some(geo.screen_cell_rect(sh, cell))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::Session;

    fn sheet_with(cells: &[(&str, &str)]) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cell, input) in cells {
            s.execute("cell.set", json!({ "cell": cell, "input": input })).unwrap();
        }
        s
    }

    fn link(s: &Session, a1: &str) -> Option<Link> {
        link_of(s.active().unwrap().wb.active().unwrap(), CellRef::parse(a1).unwrap())
    }

    #[test]
    fn finds_links_but_not_plain_text() {
        let mut s = sheet_with(&[
            ("A1", "hello"),
            ("A2", "https://example.com/a"),
            ("A3", "=HYPERLINK(\"https://example.org/x?q=\"\"1\"\"\", \"Click\")"),
            ("A4", "=hyperlink(B1)"),
            ("B1", "mailto:ada@example.com"),
            ("A5", "see https://example.com"),
            ("A6", "=HYPERLINK(\"https://\" & B1)"),
            ("A7", "12"),
        ]);
        s.execute("insert.link", json!({ "cell": "A8", "target": "https://example.net" })).unwrap();
        assert_eq!(link(&s, "A1"), None, "plain text has no card");
        assert_eq!(link(&s, "A5"), None, "text with an address inside isn't one");
        // A worked-out address isn't read from the formula, but its result is an address.
        assert_eq!(link(&s, "A6").unwrap(), Link { target: "https://mailto:ada@example.com".into(), source: Source::Text });
        assert_eq!(link(&s, "A7"), None);
        assert_eq!(link(&s, "A2").unwrap(), Link { target: "https://example.com/a".into(), source: Source::Text });
        assert_eq!(link(&s, "A3").unwrap(), Link { target: "https://example.org/x?q=\"1\"".into(), source: Source::Formula });
        assert_eq!(link(&s, "A4").unwrap().target, "mailto:ada@example.com");
        assert_eq!(link(&s, "A8").unwrap().source, Source::Stored);
    }

    #[test]
    fn a_javascript_link_shows_but_never_opens() {
        let s = sheet_with(&[("A1", "=HYPERLINK(\"javascript:alert(1)\", \"x\")"), ("A2", "=HYPERLINK(\"data:text/html,hi\")")]);
        for a1 in ["A1", "A2"] {
            let l = link(&s, a1).expect("the card still shows the address");
            assert!(matches!(l.kind(), LinkKind::Refused(_)), "{a1} has no Open");
            assert_eq!(heading(&l.kind()), "This link isn't opened");
        }
        let ok = sheet_with(&[("A1", "=HYPERLINK(\"https://example.com\")")]);
        assert!(matches!(link(&ok, "A1").unwrap().kind(), LinkKind::External(_)));
    }

    #[test]
    fn headings_name_the_real_site() {
        let h = |u: &str| heading(&classify(u));
        assert_eq!(h("https://www.Example.com/path?q=1"), "example.com");
        assert_eq!(h("http://bank.example@evil.example:8080/login"), "evil.example");
        assert_eq!(h("https://[2001:db8::1]:443/x"), "[2001:db8::1]");
        assert_eq!(h("mailto:ada@example.com?subject=Hi"), "ada@example.com");
        assert_eq!(h("Sheet2!A1"), "In this spreadsheet");
    }

    #[test]
    fn long_addresses_are_shortened_in_the_middle() {
        assert_eq!(middle_truncate("https://a.example", 52), "https://a.example");
        let long = format!("https://example.com/{}end", "x".repeat(80));
        let short = middle_truncate(&long, 52);
        assert_eq!(short.chars().count(), 52);
        assert!(short.starts_with("https://example.com/") && short.ends_with("xend") && short.contains('…'));
    }
}
