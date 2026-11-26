//! "@" mentions in comments, as in Google Sheets: typing "@" in the comment box lists the people
//! the file is shared with (and its owner), narrowed as you type; picking one writes "@Name".
//! When the comment is sent the people named are recorded on the server
//! (`POST api/files/<id>/mentions`), which marks the file for them in their file list until they
//! open it. The list comes from the Share data (`api/files/<id>/sharing`), so it only exists in
//! the web app, for a file saved on the server.

use std::cell::RefCell;

use egui::{Area, Key, Order, Rect, Ui, vec2};

use crate::house;

/// Most names shown in the list.
const SHOWN: usize = 6;
/// Longest "@…" being typed that still counts as a name (display names can have spaces).
const LONGEST: usize = 30;

/// (username, display name).
type Person = (String, String);

#[derive(Default)]
struct State {
    /// The open file's id, and who may be mentioned in it.
    file: Option<String>,
    people: Vec<Person>,
    /// The highlighted row of the list.
    pick: usize,
    /// What the list showed last frame (the test hook reads this): (query, names).
    shown: Option<(String, Vec<String>)>,
    /// Where the list was drawn: pressing on it takes the keyboard focus off the box, which must
    /// not make the list vanish before the click lands.
    area: Option<Rect>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Tells the list which server file is open (None: not saved on the server, no mentions). Loads
/// the people when the file changes.
#[cfg(target_arch = "wasm32")]
pub fn set_file(id: Option<String>, ctx: &egui::Context) {
    let changed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.file == id {
            return false;
        }
        s.file = id.clone();
        s.people.clear();
        true
    });
    if changed && let Some(id) = id {
        load(&id, ctx);
    }
}

#[cfg(target_arch = "wasm32")]
fn load(id: &str, ctx: &egui::Context) {
    use crate::files::{Method, fetch};
    let (id, ctx) = (id.to_string(), ctx.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let Ok((s, _, bytes)) = fetch(Method::Get, &format!("api/files/{id}/sharing"), None, None).await else { return };
        if s >= 400 {
            return;
        }
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let me = crate::account::me().map(|m| m.username).unwrap_or_default();
        let people: Vec<Person> = v["people"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p["username"].as_str()?.to_string(), p["name"].as_str()?.to_string())))
            .filter(|(u, _)| *u != me)
            .collect();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.file.as_deref() == Some(id.as_str()) {
                s.people = people;
            }
        });
        ctx.request_repaint();
    });
}

/// The "@…" being typed at the end of `text`: (where the "@" is, what follows it). The "@" must
/// start the text or follow a space or newline.
pub fn typing(text: &str) -> Option<(usize, &str)> {
    let at = text.rfind('@')?;
    if text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let query = &text[at + 1..];
    (query.chars().count() <= LONGEST && !query.contains('\n')).then_some((at, query))
}

/// People whose display name or username starts with `query` (or has a word that does).
fn matching<'a>(people: &'a [Person], query: &str) -> Vec<&'a Person> {
    let q = query.to_lowercase();
    people
        .iter()
        .filter(|(u, n)| {
            let (u, n) = (u.to_lowercase(), n.to_lowercase());
            u.starts_with(&q) || n.starts_with(&q) || n.split_whitespace().any(|w| w.starts_with(&q))
        })
        .take(SHOWN)
        .collect()
}

/// `text` with the "@…" being typed replaced by "@Name ".
fn insert(text: &str, at: usize, name: &str) -> String {
    format!("{}@{name} ", &text[..at])
}

/// Usernames of the people named in `text` as "@Name" (the name, then a space, punctuation or the end).
fn named(text: &str, people: &[Person]) -> Vec<String> {
    people
        .iter()
        .filter(|(_, n)| {
            let tag = format!("@{n}");
            text.match_indices(&tag).any(|(i, _)| {
                let before = text[..i].chars().next_back();
                let after = text[i + tag.len()..].chars().next();
                before.is_none_or(char::is_whitespace) && after.is_none_or(|c| !c.is_alphanumeric())
            })
        })
        .map(|(u, _)| u.clone())
        .collect()
}

/// The list under the comment box (`field`), while an "@name" is being typed in `text`. Picking
/// a row (click, or Tab / Enter with the arrows) rewrites `text`; returns whether it did.
pub fn list(ui: &mut Ui, field: Rect, has_focus: bool, text: &mut String) -> bool {
    let (rows, at, query) = STATE.with(|s| {
        let s = s.borrow();
        let Some((at, query)) = typing(text) else { return (Vec::new(), 0, String::new()) };
        (matching(&s.people, query).into_iter().cloned().collect::<Vec<_>>(), at, query.to_string())
    });
    let on_list = STATE
        .with(|s| s.borrow().area)
        .is_some_and(|a| ui.input(|i| i.pointer.any_down() && i.pointer.interact_pos().is_some_and(|p| a.contains(p))));
    if rows.is_empty() || !(has_focus || on_list) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.shown = None;
            s.area = None;
        });
        return false;
    }
    let mut chosen: Option<usize> = None;
    let pick = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.pick = s.pick.min(rows.len() - 1);
        s.shown = Some((query.clone(), rows.iter().map(|(_, n)| n.clone()).collect()));
        s.pick
    });
    // Keys are taken before the box sees them (Enter would add a line).
    let (down, up, enter) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::Enter) | i.consume_key(egui::Modifiers::NONE, Key::Tab),
        )
    });
    let mut pick = pick;
    if down {
        pick = (pick + 1) % rows.len();
    }
    if up {
        pick = (pick + rows.len() - 1) % rows.len();
    }
    if enter {
        chosen = Some(pick);
    }
    let shown = Area::new(egui::Id::unique("starsheets_mention_list")).order(Order::Tooltip).fixed_pos(field.left_bottom() + vec2(0.0, 2.0)).show(
        ui.ctx(),
        |ui| {
            house::card_frame().show(ui, |ui| {
                ui.set_width(field.width().max(180.0));
                for (i, (_, name)) in rows.iter().enumerate() {
                    // Picked on the press: the box loses focus then, and the list with it.
                    let row = ui.selectable_label(i == pick, name);
                    if row.clicked() || (row.hovered() && ui.input(|i| i.pointer.primary_pressed())) {
                        chosen = Some(i);
                    }
                }
            });
        },
    );
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.pick = pick;
        s.area = Some(shown.response.rect);
    });
    if let Some(i) = chosen {
        *text = insert(text, at, &rows[i].1);
        STATE.with(|s| s.borrow_mut().shown = None);
        return true;
    }
    false
}

/// A comment was sent: record the people it names (a failure is ignored; the comment is saved
/// either way).
pub fn record(text: &str) {
    let (file, users) = STATE.with(|s| {
        let s = s.borrow();
        (s.file.clone(), named(text, &s.people))
    });
    let Some(file) = file.filter(|_| !users.is_empty()) else { return };
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        use crate::files::{Method, fetch};
        let body = serde_json::to_vec(&serde_json::json!({ "users": users })).unwrap_or_default();
        let _ = fetch(Method::Post, &format!("api/files/{file}/mentions"), Some(body), None).await;
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (file, users);
}

#[cfg(target_arch = "wasm32")]
/// For the test hook: who could be mentioned, and what the list shows now.
pub fn test_state() -> serde_json::Value {
    STATE.with(|s| {
        let s = s.borrow();
        serde_json::json!({
            "people": s.people.iter().map(|(u, n)| serde_json::json!({ "username": u, "name": n })).collect::<Vec<_>>(),
            "open": s.shown.is_some(),
            "query": s.shown.as_ref().map(|(q, _)| q),
            "names": s.shown.as_ref().map(|(_, n)| n),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> Vec<Person> {
        vec![("ann".into(), "Ann Lee".into()), ("vic".into(), "Vic".into()), ("bob".into(), "Bob Lee".into())]
    }

    #[test]
    fn at_sign_opens_the_list_only_at_a_word_start() {
        assert_eq!(typing("hi @A"), Some((3, "A")));
        assert_eq!(typing("@"), Some((0, "")));
        assert_eq!(typing("mail me@x"), None);
        assert_eq!(typing("hi @Ann Lee ok"), Some((3, "Ann Lee ok")));
        assert_eq!(typing("no at sign"), None);
    }

    #[test]
    fn the_list_narrows_as_you_type() {
        let p = people();
        assert_eq!(matching(&p, "").len(), 3);
        assert_eq!(matching(&p, "l").iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["ann", "bob"]);
        assert_eq!(matching(&p, "v")[0].0, "vic");
        assert!(matching(&p, "zz").is_empty());
    }

    #[test]
    fn picking_writes_the_name_and_sending_names_the_people() {
        let t = insert("thanks @An", 7, "Ann Lee");
        assert_eq!(t, "thanks @Ann Lee ");
        assert_eq!(named(&t, &people()), ["ann"]);
        assert_eq!(named("@Vic, look", &people()), ["vic"]);
        assert!(named("@Vickers", &people()).is_empty());
        assert!(named("email@Vic", &people()).is_empty());
    }
}
