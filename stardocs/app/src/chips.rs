//! Smart chips, as in Google Docs: typing "@" opens a small picker of people (accounts on this
//! Starbase server, `GET api/users`), files (the Drive files you can see, `GET api/files`) and
//! dates (Today, Tomorrow, or one picked from a calendar). Typing filters it, ↑/↓ choose, Enter or
//! Tab inserts, Esc closes. The chip is the item's text on a grey background:
//! - a person: "@Name", hovering shows their name and username;
//! - a file: its name, a link to the file (Ctrl/⌘+click opens it, as every link in the text);
//! - a date: "25 Nov 2026", hovering shows the full date.
//!
//! People and dates carry a `chip:<kind>:<tip>` link (WordCraft fork, StarDocs 0012): never
//! followed, and saved to Word as plain text. A file chip is an ordinary hyperlink, so .docx keeps
//! it as one. Reading a .docx back gives plain text and links: chips are not rebuilt (yet).

use egui::{Color32, Key, Modifiers, RichText, vec2};
use serde_json::json;
use wordcraft_doc::Pos;
use wordcraft_ui_egui::WordApp;

use crate::shell::{BLUE, TEXT, TEXT_DIM};

/// The chip's background (Google Docs' chip grey).
pub const CHIP_GREY: &str = "E8EAED";
/// Typing more than this after "@" closes the picker: it is a word, not a search.
const MAX_QUERY: usize = 40;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// A calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Date {
    /// Days since 1970-01-01 (H. Hinnant's days_from_civil).
    fn days(self) -> i64 {
        let y = i64::from(self.y) - i64::from(self.m <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (i64::from(self.m) + 9) % 12;
        let doy = (153 * mp + 2) / 5 + i64::from(self.d) - 1;
        era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
    }

    fn from_days(z: i64) -> Date {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Date { y: (yoe + era * 400 + i64::from(m <= 2)) as i32, m, d }
    }

    pub fn plus(self, days: i64) -> Date {
        Date::from_days(self.days() + days)
    }

    /// 0 = Monday.
    fn weekday(self) -> usize {
        (self.days() + 3).rem_euclid(7) as usize
    }

    /// "25 Nov 2026": the chip's text.
    pub fn short(self) -> String {
        format!("{} {} {}", self.d, &MONTHS[self.m as usize - 1][..3], self.y)
    }

    /// "Wednesday, 25 November 2026": the hover tip.
    pub fn long(self) -> String {
        format!("{}, {} {} {}", DAYS[self.weekday()], self.d, MONTHS[self.m as usize - 1], self.y)
    }

    fn month_len(self) -> u32 {
        Date { y: self.y + i32::from(self.m == 12), m: self.m % 12 + 1, d: 1 }.days().abs_diff(Date { d: 1, ..self }.days()) as u32
    }
}

/// Today, by this computer's clock.
fn today() -> Date {
    #[cfg(target_arch = "wasm32")]
    {
        let t = js_sys::Date::new_0();
        Date { y: t.get_full_year() as i32, m: t.get_month() + 1, d: t.get_date() }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use chrono::Datelike;
        let t = chrono::Local::now().date_naive();
        Date { y: t.year(), m: t.month(), d: t.day() }
    }
}

/// One line of the picker.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code, reason = "people and files come from the web app's server"))]
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Person {
        name: String,
        username: String,
    },
    File {
        id: String,
        name: String,
    },
    Date {
        label: &'static str,
        date: Date,
    },
    /// "Pick a date…": opens the calendar.
    Calendar,
}

impl Item {
    fn section(&self) -> &'static str {
        match self {
            Item::Person { .. } => "People",
            Item::File { .. } => "Files",
            Item::Date { .. } | Item::Calendar => "Dates",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Item::Person { .. } => "👤",
            Item::File { name, .. } if is_doc(name) => "📄",
            Item::File { name, .. } if name.to_lowercase().ends_with(".pptx") => "📊",
            Item::File { .. } => "📗",
            Item::Date { .. } | Item::Calendar => "📅",
        }
    }

    fn label(&self) -> String {
        match self {
            Item::Person { name, username } => format!("{name}  @{username}"),
            Item::File { name, .. } => name.clone(),
            Item::Date { label, date } => format!("{label}  {}", date.short()),
            Item::Calendar => "Pick a date…".into(),
        }
    }

    /// Whether the picker shows this item while `query` is typed after "@".
    fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        let has = |s: &str| s.to_lowercase().contains(&q);
        q.is_empty()
            || match self {
                Item::Person { name, username } => has(name) || has(username),
                Item::File { name, .. } => has(name),
                Item::Date { label, date } => has(label) || has(&date.short()) || "date".starts_with(&q),
                Item::Calendar => "pick a date".contains(&q) || "date".starts_with(&q) || "calendar".starts_with(&q),
            }
    }

    /// The chip's text and link, or `None` for an item that isn't a chip (the calendar).
    /// `base` is the address the Starbase home page is at (ends in '/').
    pub fn chip(&self, base: &str) -> Option<(String, String)> {
        Some(match self {
            Item::Person { name, username } => (format!("@{name}"), format!("chip:person:{name} (@{username})")),
            Item::File { id, name } => (file_title(name), file_url(base, id, name)?),
            Item::Date { date, .. } => (date.short(), format!("chip:date:{}", date.long())),
            Item::Calendar => return None,
        })
    }
}

fn is_doc(name: &str) -> bool {
    let n = name.to_lowercase();
    [".docx", ".docm", ".dotx", ".odt", ".rtf", ".md", ".txt"].iter().any(|e| n.ends_with(e))
}

/// A file's name without its extension, as Drive shows it.
fn file_title(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}

/// The page a file opens on: Docs for a document, Slides for a deck, the spreadsheet page
/// otherwise (as the home page's own links). Only a plain id is accepted, so the link can only
/// point at this server's own pages.
pub fn file_url(base: &str, id: &str, name: &str) -> Option<String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let page = if is_doc(name) {
        "docs/"
    } else if name.to_lowercase().ends_with(".pptx") {
        "slides/"
    } else {
        ""
    };
    Some(format!("{base}{page}?file={id}"))
}

/// The picker's items for `query`, in order: people, files, dates. At most 6 of each list.
pub fn filtered(people: &[Item], files: &[Item], today: Date, query: &str) -> Vec<Item> {
    let dates = [Item::Date { label: "Today", date: today }, Item::Date { label: "Tomorrow", date: today.plus(1) }, Item::Calendar];
    let mut out: Vec<Item> = people.iter().filter(|i| i.matches(query)).take(6).cloned().collect();
    out.extend(files.iter().filter(|i| i.matches(query)).take(6).cloned());
    out.extend(dates.into_iter().filter(|i| i.matches(query)));
    out
}

/// Puts `item`'s chip in place of the text from `at` (the "@") to `caret`, then a space, with
/// the caret after it. The chip is grey and carries its link; the space doesn't.
pub fn insert(app: &mut WordApp, at: &Pos, caret: &Pos, item: &Item, base: &str) -> Result<(), String> {
    let (text, link) = item.chip(base).ok_or("not a chip")?;
    app.run("select.range", json!({"anchor": at, "focus": caret}))?;
    app.run("text.insert", json!({"text": format!("{text} "), "raw": true}))?;
    let end = Pos { off: at.off + text.len(), ..at.clone() };
    app.run("select.range", json!({"anchor": at, "focus": end}))?;
    app.run("format.shading", json!({"color": CHIP_GREY}))?;
    app.run("format.set", json!({"props": {"link": link}}))?;
    app.run("caret.set", json!({"pos": Pos { off: end.off + 1, ..end }}))?;
    Ok(())
}

/// The picker's state, kept by the shell.
#[derive(Default)]
pub struct Chips {
    /// An "@" was typed this frame: next frame, see whether it opens the picker.
    armed: bool,
    /// The "@" the open picker belongs to.
    at: Option<Pos>,
    selected: usize,
    /// The month the calendar shows, while it is open.
    calendar: Option<Date>,
    people: Vec<Item>,
    files: Vec<Item>,
    #[cfg(target_arch = "wasm32")]
    loaded: bool,
    #[cfg(target_arch = "wasm32")]
    inbox: std::rc::Rc<std::cell::RefCell<Vec<(bool, Vec<Item>)>>>,
}

impl Chips {
    /// What is typed after the "@", while the picker belongs to it; `None` when the caret has left.
    fn query(&self, app: &WordApp) -> Option<String> {
        let at = self.at.as_ref()?;
        let caret = &app.session.sel.focus;
        if app.session.sel.anchor != *caret || caret.story != at.story || caret.path != at.path || caret.off <= at.off {
            return None;
        }
        let text = &app.session.doc.para_at(at)?.text;
        let q = text.get(at.off..caret.off)?.strip_prefix('@')?;
        (q.len() <= MAX_QUERY && !q.contains("  ") && !q.contains(['\n', '\u{FFFC}'])).then(|| q.to_string())
    }

    /// Every frame, before WordCraft's own: opens the picker after an "@" typed at the start of a
    /// word, takes its keys, and draws it.
    pub fn ui(&mut self, app: &mut WordApp, ctx: &egui::Context, editable: bool) {
        #[cfg(target_arch = "wasm32")]
        for (people, items) in std::mem::take(&mut *self.inbox.borrow_mut()) {
            if people { self.people = items } else { self.files = items }
        }
        if std::mem::take(&mut self.armed) && editable && self.at.is_none() {
            self.open_at_caret(app, ctx);
        }
        self.armed = editable && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(t) if t.ends_with('@'))));
        if self.armed {
            ctx.request_repaint();
        }
        let Some(query) = self.query(app) else {
            self.close();
            return;
        };
        let items = filtered(&self.people, &self.files, today(), &query);
        if items.is_empty() && self.calendar.is_none() && query.chars().count() > 3 {
            self.close();
            return;
        }
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let (esc, up, down, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab),
            )
        });
        if esc {
            self.close();
            return;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if down && self.selected + 1 < items.len() {
            self.selected += 1;
        }
        let mut chosen = enter.then(|| items.get(self.selected).cloned()).flatten();
        let pos = wordcraft_ui_egui::canvas::caret_screen(app).map_or(egui::pos2(200.0, 200.0), |(p, h)| p + vec2(0.0, h + 4.0));
        egui::Area::new(egui::Id::unique("stardocs_chips")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(Color32::WHITE).corner_radius(8.0).show(ui, |ui| {
                ui.set_width(300.0);
                if let Some(month) = self.calendar {
                    if let Some(d) = self.calendar_ui(ui, month) {
                        chosen = Some(Item::Date { label: "", date: d });
                    }
                    return;
                }
                if items.is_empty() {
                    ui.label(RichText::new("No matches").color(TEXT_DIM));
                }
                let mut section = "";
                for (n, item) in items.iter().enumerate() {
                    if item.section() != section {
                        section = item.section();
                        ui.label(RichText::new(section).size(11.0).color(TEXT_DIM));
                    }
                    let text = RichText::new(format!("{}  {}", item.icon(), item.label())).color(TEXT);
                    let r = ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(n == self.selected, text).frame_when_inactive(false));
                    if n == self.selected {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        chosen = Some(item.clone());
                    }
                }
            });
        });
        match chosen {
            Some(Item::Calendar) => self.calendar = Some(today()),
            Some(item) => {
                if let (Some(at), caret) = (self.at.clone(), app.session.sel.focus.clone()) {
                    let _ = insert(app, &at, &caret, &item, &home_base());
                }
                self.close();
            }
            None => {}
        }
    }

    /// The month grid; returns the day clicked.
    fn calendar_ui(&mut self, ui: &mut egui::Ui, month: Date) -> Option<Date> {
        let first = Date { d: 1, ..month };
        let mut picked = None;
        ui.horizontal(|ui| {
            if ui.button("‹").clicked() {
                self.calendar = Some(first.plus(-1));
            }
            ui.label(RichText::new(format!("{} {}", MONTHS[month.m as usize - 1], month.y)).strong().color(TEXT));
            if ui.button("›").clicked() {
                self.calendar = Some(first.plus(i64::from(first.month_len())));
            }
        });
        let today = today();
        egui::Grid::new("stardocs_chip_cal").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for d in DAYS {
                ui.label(RichText::new(&d[..2]).size(11.0).color(TEXT_DIM));
            }
            ui.end_row();
            for _ in 0..first.weekday() {
                ui.label("");
            }
            for d in 1..=first.month_len() {
                let day = Date { d, ..first };
                let mut text = RichText::new(d.to_string()).color(TEXT);
                if day == today {
                    text = text.color(BLUE).strong();
                }
                if ui.add(egui::Button::new(text).min_size(vec2(36.0, 26.0)).frame_when_inactive(false)).clicked() {
                    picked = Some(day);
                }
                if day.weekday() == 6 {
                    ui.end_row();
                }
            }
        });
        picked
    }

    fn open_at_caret(&mut self, app: &WordApp, ctx: &egui::Context) {
        let s = &app.session;
        if s.sel.anchor != s.sel.focus || s.sel.focus.off == 0 {
            return;
        }
        let at = Pos { off: s.sel.focus.off - 1, ..s.sel.focus.clone() };
        let Some(para) = s.doc.para_at(&at) else { return };
        let before = para.text.get(..at.off).and_then(|t| t.chars().last());
        if para.text.get(at.off..s.sel.focus.off) != Some("@") || before.is_some_and(|c| !c.is_whitespace() && !"([{\"'".contains(c)) {
            return;
        }
        (self.at, self.calendar, self.selected) = (Some(at), None, 0);
        self.load(ctx);
    }

    fn close(&mut self) {
        (self.at, self.calendar, self.selected) = (None, None, 0);
    }

    /// For tests: open the picker at the "@" just before the caret.
    #[cfg(test)]
    pub fn open_for_test(&mut self, app: &WordApp) {
        let f = &app.session.sel.focus;
        self.at = Some(Pos { off: f.off - 1, ..f.clone() });
    }

    /// People and files, once per page load, from the server (the web app on a Starbase server).
    #[cfg(target_arch = "wasm32")]
    fn load(&mut self, ctx: &egui::Context) {
        if std::mem::replace(&mut self.loaded, true) {
            return;
        }
        for people in [true, false] {
            let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let url = crate::server::api(if people { "api/users" } else { "api/files" });
                let Ok((200, _, bytes)) = crate::server::fetch("GET", &url, None, None).await else { return };
                let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
                let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let items = if people {
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .map(|p| Item::Person {
                            name: Some(s(p, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| s(p, "username")),
                            username: s(p, "username"),
                        })
                        .filter(|i| matches!(i, Item::Person { username, .. } if !username.is_empty()))
                        .collect()
                } else {
                    v.get("files")
                        .and_then(|f| f.as_array())
                        .into_iter()
                        .flatten()
                        .map(|f| Item::File { id: s(f, "id"), name: s(f, "name") })
                        .filter(|i| matches!(i, Item::File { id, name } if !id.is_empty() && !name.is_empty()))
                        .collect()
                };
                inbox.borrow_mut().push((people, items));
                ctx.request_repaint();
            });
        }
    }

    /// The desktop program offers dates only (its server connection has no people or file list yet).
    #[cfg(not(target_arch = "wasm32"))]
    fn load(&mut self, _ctx: &egui::Context) {}
}

/// The Starbase home page's address (where `api/` is), ending in '/'.
fn home_base() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(loc) = web_sys::window().map(|w| w.location()) else { return String::new() };
        let (origin, path) = (loc.origin().unwrap_or_default(), loc.pathname().unwrap_or_default());
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        // StarDocs sits in docs/ beside the home page (server.rs `api`).
        let home = dir.strip_suffix("docs/").unwrap_or(dir);
        format!("{origin}{home}")
    }
    #[cfg(not(target_arch = "wasm32"))]
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_ui_egui::Services;

    fn person(name: &str, user: &str) -> Item {
        Item::Person { name: name.into(), username: user.into() }
    }

    fn file(id: &str, name: &str) -> Item {
        Item::File { id: id.into(), name: name.into() }
    }

    const DAY: Date = Date { y: 2026, m: 11, d: 25 };

    #[test]
    fn dates_count_and_read_right() {
        assert_eq!(DAY.long(), "Wednesday, 25 November 2026");
        assert_eq!(DAY.plus(6), Date { y: 2026, m: 12, d: 1 });
        assert_eq!(Date { y: 2028, m: 2, d: 28 }.plus(1).short(), "29 Feb 2028");
        assert_eq!(Date { y: 2026, m: 12, d: 31 }.plus(1), Date { y: 2027, m: 1, d: 1 });
        assert_eq!(Date { y: 2026, m: 2, d: 1 }.month_len(), 28);
        assert_eq!(Date { y: 2026, m: 12, d: 9 }.month_len(), 31);
    }

    #[test]
    fn typing_filters_the_picker() {
        let people = [person("Ann Lee", "ann"), person("Bob Stone", "bstone")];
        let files = [file("f1", "Budget 2026.xlsx"), file("f2", "Annual report.docx")];
        let all = filtered(&people, &files, DAY, "");
        assert_eq!(all.len(), 7, "{all:?}");
        let ann = filtered(&people, &files, DAY, "ANN");
        assert_eq!(ann, vec![people[0].clone(), files[1].clone()]);
        assert_eq!(filtered(&people, &files, DAY, "bst"), vec![people[1].clone()]);
        assert_eq!(filtered(&people, &files, DAY, "tom"), vec![Item::Date { label: "Tomorrow", date: DAY.plus(1) }]);
        assert!(filtered(&people, &files, DAY, "date").contains(&Item::Calendar));
        assert!(filtered(&people, &files, DAY, "zzz").is_empty());
    }

    #[test]
    fn file_links_go_to_the_right_page_and_only_for_plain_ids() {
        assert_eq!(file_url("https://h/", "a1", "Notes.docx").as_deref(), Some("https://h/docs/?file=a1"));
        assert_eq!(file_url("https://h/", "a1", "Deck.pptx").as_deref(), Some("https://h/slides/?file=a1"));
        assert_eq!(file_url("https://h/", "a1", "Sums.xlsx").as_deref(), Some("https://h/?file=a1"));
        assert_eq!(file_url("https://h/", "a/../x", "Notes.docx"), None);
    }

    fn app_with(text: &str) -> WordApp {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::blank_document(&Default::default())), Services::default());
        app.run("text.insert", json!({"text": text})).unwrap();
        app
    }

    fn chip_in(app: &mut WordApp, item: &Item) {
        let mut chips = Chips::default();
        chips.open_for_test(app);
        let caret = app.session.sel.focus.clone();
        insert(app, chips.at.as_ref().unwrap(), &caret, item, "https://h/").unwrap();
    }

    #[test]
    fn choosing_puts_a_grey_chip_in_place_of_the_at_sign() {
        let mut app = app_with("Ask @");
        chip_in(&mut app, &person("Ann Lee", "ann"));
        let para = app.session.doc.para_at(&app.session.sel.focus).unwrap().clone();
        assert_eq!(para.text, "Ask @Ann Lee ");
        assert_eq!(app.session.sel.focus.off, para.text.len(), "caret after the chip's space");
        let chip = para.props_of_char(5);
        assert_eq!(chip.link.as_deref(), Some("chip:person:Ann Lee (@ann)"));
        assert_eq!(chip.shading, Some(wordcraft_doc::props::Rgb(0xE8, 0xEA, 0xED)));
        let space = para.props_of_char(para.text.len() - 1);
        assert_eq!((space.link.as_ref(), space.shading), (None, None), "the space after is plain");
        assert_eq!(para.props_of_char(0).link, None);
    }

    #[test]
    fn the_typed_query_is_replaced_too() {
        let mut app = app_with("See @rep");
        let at = Pos { off: 4, ..app.session.sel.focus.clone() };
        let caret = app.session.sel.focus.clone();
        insert(&mut app, &at, &caret, &file("f2", "Annual report.docx"), "https://h/").unwrap();
        let para = app.session.doc.para_at(&app.session.sel.focus).unwrap();
        assert_eq!(para.text, "See Annual report ");
        assert_eq!(para.props_of_char(4).link.as_deref(), Some("https://h/docs/?file=f2"));
    }

    #[test]
    fn word_gets_plain_text_for_people_and_dates_and_a_link_for_files() {
        let mut app = app_with("@");
        chip_in(&mut app, &person("Ann Lee", "ann"));
        app.run("text.insert", json!({"text": "@"})).unwrap();
        chip_in(&mut app, &file("f2", "Annual report.docx"));
        app.run("text.insert", json!({"text": "@"})).unwrap();
        chip_in(&mut app, &Item::Date { label: "Today", date: DAY });
        let bytes = wordcraft_engine::io::save_bytes_with("chips.docx", &app.session.doc, None, &Default::default()).unwrap();
        let back = wordcraft_engine::io::open_bytes("chips.docx", &bytes).unwrap();
        let session = wordcraft_engine::Session::new(back);
        let para = session.doc.para_at(&session.sel.focus).unwrap();
        assert_eq!(para.text, "@Ann Lee Annual report 25 Nov 2026 ");
        let links: Vec<Option<String>> = [0, 10, 24].iter().map(|&o| para.props_of_char(o).link.clone()).collect();
        assert_eq!(links, vec![None, Some("https://h/docs/?file=f2".into()), None]);
    }
}
