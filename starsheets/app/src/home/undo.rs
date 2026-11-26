//! Undo in the Drive (to-do 3 row 16): moving, renaming, New folder and Move to bin can be undone
//! with Ctrl+Z or the "Undo" in the note that shows after a move or a delete, as in Google Drive.
//!
//! Every undo is a new request to the server that says what it expects to find, so it never
//! guesses: the item must still be where the action left it (`from=` for files, the folder's path
//! for folders), the old name must still be free where it goes back to (`as=` / `exact=1`: no
//! "name (2)"), a new folder is only binned while it is still empty (`if_empty=1`), and a binned
//! item comes back only to exactly where it was (`expect=&exact=1`). The server's usual access
//! checks apply too. When something has changed since, nothing is touched and the note says why.

use super::*;

/// The most actions kept for Ctrl+Z.
const KEEP: usize = 30;
/// How long the note after a move or a delete stays (seconds).
const NOTE_SECS: f64 = 8.0;

/// Works out an action's step from the server's answer (None: it did nothing).
pub(super) type Then = Box<dyn FnOnce(&Json) -> Option<Step>>;

/// One thing done to one item, and so how to undo it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// A file moved or renamed: `id` is now at path `now`, it was at `was`.
    File { id: String, was: String, now: String },
    /// A folder moved or renamed from `was` to `now`; `id` (the server's, "" on this computer)
    /// makes sure an undo moves this folder, not another that has taken its name since.
    Folder { id: String, was: String, now: String },
    /// A folder made with New folder.
    Made { path: String },
    /// Something moved to the bin: its name in the bin, and where it was.
    Binned { bin: String, original: String },
    /// Photos: a picture's star was turned `on` (undo: back to the other).
    Star { id: String, on: bool },
    /// Photos: these pictures were added to an album (undo: take them out again).
    AlbumAdd { album: String, ids: Vec<String> },
    /// Photos: these pictures were taken out of an album (undo: put them back).
    AlbumRemove { album: String, ids: Vec<String> },
    /// Photos: a picture was turned a quarter to the left (or right); undo: the other way.
    Rotate { id: String, left: bool },
}

impl Step {
    /// The request that undoes this step, checked by the server (see the top of this file).
    pub(super) fn undo_request(&self) -> (Method, String, Option<Vec<u8>>) {
        let files = |ids: &[String]| Some(serde_json::json!({ "files": ids }).to_string().into_bytes());
        match self {
            Step::Star { id, on } => (Method::Post, format!("api/files/{id}/star"), Some(format!("{{\"on\":{}}}", !on).into_bytes())),
            Step::Rotate { id, left } => (Method::Post, format!("api/photos/rotate/{id}"), Some(rotate_body(!left))),
            Step::AlbumAdd { album, ids } => (Method::Post, format!("api/photos/albums/{}/remove", enc(album)), files(ids)),
            Step::AlbumRemove { album, ids } => (Method::Post, format!("api/photos/albums/{}/add", enc(album)), files(ids)),
            _ => {
                let (m, u) = self.drive_request();
                (m, u, None)
            }
        }
    }

    /// The Drive steps' requests (no body).
    fn drive_request(&self) -> (Method, String) {
        let (was_parent, was_name) = (|p: &str| parent_of(p).to_string(), |p: &str| name_of(p).to_string());
        match self {
            Step::File { id, was, now } if parent_of(was) == parent_of(now) => {
                (Method::Post, format!("api/files/{id}/rename?name={}&from={}&exact=1", enc(&was_name(was)), enc(now)))
            }
            Step::File { id, was, now } => {
                (Method::Post, format!("api/files/{id}/move?folder={}&from={}&as={}", enc(&was_parent(was)), enc(now), enc(&was_name(was))))
            }
            Step::Folder { id, was, now } if parent_of(was) == parent_of(now) => {
                (Method::Post, format!("api/folders/rename?path={}&name={}&exact=1{}", enc(now), enc(&was_name(was)), id_q(id)))
            }
            Step::Folder { id, was, now } => {
                (Method::Post, format!("api/folders/move?path={}&to={}&as={}{}", enc(now), enc(&was_parent(was)), enc(&was_name(was)), id_q(id)))
            }
            Step::Made { path } => (Method::Delete, format!("api/folders?path={}&if_empty=1", enc(path))),
            Step::Binned { bin, original } => (Method::Post, format!("api/bin/restore?name={}&expect={}&exact=1", enc(bin), enc(original))),
            Step::Star { .. } | Step::AlbumAdd { .. } | Step::AlbumRemove { .. } | Step::Rotate { .. } => {
                unreachable!("they have a body: see undo_request")
            }
        }
    }

    /// Where the item is now (for the message when it can't be found there).
    fn label(&self) -> String {
        match self {
            Step::File { now, .. } | Step::Folder { now, .. } => name_of(now).to_string(),
            Step::Made { path } => name_of(path).to_string(),
            Step::Binned { original, .. } => name_of(original).to_string(),
            Step::Star { .. } => "the star".into(),
            Step::AlbumAdd { .. } | Step::AlbumRemove { .. } => "the album".into(),
            Step::Rotate { .. } => "the picture".into(),
        }
    }
}

/// The body of a rotate request.
pub(super) fn rotate_body(left: bool) -> Vec<u8> {
    format!("{{\"dir\":\"{}\"}}", if left { "left" } else { "right" }).into_bytes()
}

/// `&id=…` for a folder's undo, when the server gave its id.
fn id_q(id: &str) -> String {
    if id.is_empty() { String::new() } else { format!("&id={}", enc(id)) }
}

/// What the person did in one go (one drag of three files is one action).
#[derive(Clone, Debug)]
pub(super) struct Action {
    pub(super) steps: Vec<Step>,
    /// "Moved to Reports", "Renamed", …
    pub(super) label: String,
    serial: u64,
}

/// An action whose requests haven't all been answered yet.
pub(super) struct Waiting {
    label: String,
    left: usize,
    steps: Vec<Step>,
    /// Show the note with Undo when it's done (moves and deletes, as in Google Drive).
    note: bool,
    /// The naming of a folder just made: it becomes part of that New folder's undo.
    naming: bool,
}

/// The note at the bottom: what was done, and Undo for the action `serial` while it's the last.
pub(super) struct Note {
    pub(super) text: String,
    serial: Option<u64>,
    until: f64,
}

#[derive(Default)]
pub(super) struct Undo {
    pub(super) done: Vec<Action>,
    pub(super) waiting: HashMap<u64, Waiting>,
    /// Requests of the undo running now still to be answered, and what went wrong so far.
    pub(super) undoing: Option<(usize, Vec<String>, String)>,
    pub(super) note: Option<Note>,
    next: u64,
}

impl Undo {
    /// Starts an action of `count` requests; each answer comes back as `Reply::Did(group, …)`.
    pub(super) fn start(&mut self, label: String, count: usize, note: bool) -> u64 {
        self.next += 1;
        self.waiting.insert(self.next, Waiting { label, left: count, steps: Vec::new(), note, naming: false });
        self.next
    }

    pub(super) fn start_naming(&mut self) -> u64 {
        let g = self.start(String::new(), 1, false);
        if let Some(w) = self.waiting.get_mut(&g) {
            w.naming = true;
        }
        g
    }

    /// One request of action `group` answered (`step`: what it did, None if it failed or did nothing).
    pub(super) fn answered(&mut self, group: u64, step: Option<Step>, now: f64) {
        let Some(w) = self.waiting.get_mut(&group) else { return };
        w.left = w.left.saturating_sub(1);
        if let Some(s) = step {
            w.steps.push(s);
        }
        if w.left > 0 {
            return;
        }
        let w = self.waiting.remove(&group).expect("there");
        if w.naming {
            // The New folder's own name: undoing it removes the folder (as it is named now).
            if let (Some(Step::Folder { was, now: named, .. }), Some(top)) = (w.steps.first(), self.done.last_mut())
                && top.steps == [Step::Made { path: was.clone() }]
            {
                top.steps = vec![Step::Made { path: named.clone() }];
            }
            return;
        }
        if w.steps.is_empty() {
            return;
        }
        let serial = group;
        self.done.push(Action { steps: w.steps, label: w.label.clone(), serial });
        if self.done.len() > KEEP {
            self.done.remove(0);
        }
        if w.note {
            self.note = Some(Note { text: w.label, serial: Some(serial), until: now + NOTE_SECS });
        }
    }

    /// A Photos action that is already done (its steps known): kept for Ctrl+Z; Photos shows its own note.
    pub(super) fn record(&mut self, label: String, steps: Vec<Step>) {
        if steps.is_empty() {
            return;
        }
        self.next += 1;
        self.done.push(Action { steps, label, serial: self.next });
        if self.done.len() > KEEP {
            self.done.remove(0);
        }
    }

    /// A New folder, undone by removing it while it's empty.
    pub(super) fn made(&mut self, path: String) {
        self.next += 1;
        self.done.push(Action { steps: vec![Step::Made { path }], label: "New folder".into(), serial: self.next });
    }

    fn say(&mut self, text: String, now: f64) {
        self.note = Some(Note { text, serial: None, until: now + NOTE_SECS });
    }
}

impl Home {
    /// Ctrl+Z, or Undo in the note: undoes the last action (all of its items), newest step first.
    pub(super) fn undo(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.undo.undoing.is_some() || !self.undo.waiting.is_empty() {
            self.undo.say("Still working on the last change\u{2026}".into(), now);
            return;
        }
        let Some(action) = self.undo.done.pop() else {
            self.undo.say("Nothing to undo.".into(), now);
            return;
        };
        self.undo.note = None;
        self.undo.undoing = Some((action.steps.len(), Vec::new(), action.label));
        for step in action.steps.into_iter().rev() {
            let (method, url, body) = step.undo_request();
            let label = step.label();
            let turned = if let Step::Rotate { id, .. } = &step { Some(id.clone()) } else { None };
            call_any(&self.store, &self.inbox, ctx, method, url, body, move |r| {
                // A turned picture's new thumbnail version, so the grid and viewer redraw it.
                Reply::Undone(r.map(|b| turned.map(|id| (id, serde_json::from_slice::<serde_json::Value>(&b).ok().and_then(|v| v["thumb_v"].as_str().map(str::to_string))))).map_err(|(status, msg)| match status {
                    404 => format!("\u{201c}{label}\u{201d} isn't where it was left any more, so it was left as it is."),
                    _ => msg,
                }))
            });
        }
    }

    /// One undo request answered.
    pub(super) fn undone(&mut self, ctx: &egui::Context, r: Result<Option<(String, Option<String>)>, String>) {
        let now = ctx.input(|i| i.time);
        let r = r.map(|turned| {
            if let Some((id, v)) = turned {
                self.photos.edited(&id, v);
            }
        });
        let Some((left, errors, _)) = self.undo.undoing.as_mut() else { return };
        *left -= 1;
        if let Err(e) = r {
            errors.push(e);
        }
        if *left > 0 {
            return;
        }
        let (_, errors, label) = self.undo.undoing.take().expect("there");
        if errors.is_empty() {
            self.undo.say(format!("Undone: {label}"), now);
        } else {
            self.error = Some(format!("Couldn't undo all of it: {}", errors.join(" ")));
        }
        self.refresh(ctx);
    }

    /// The note at the bottom left, as Google Drive's: "Moved to X   Undo".
    pub(super) fn note_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some(note) = &self.undo.note else { return };
        if now > note.until {
            self.undo.note = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((note.until - now).max(0.05)));
        let can_undo = note.serial.is_some() && self.undo.done.last().map(|a| a.serial) == note.serial;
        let text = note.text.clone();
        let screen = ctx.content_rect();
        let mut clicked = false;
        let mut closed = false;
        egui::Area::new(egui::Id::unique("home_note"))
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::pos2(screen.left() + house::XL, screen.bottom() - house::XL - 48.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(house::TIP_BG)
                    .corner_radius(house::PANEL_RADIUS)
                    .inner_margin(egui::Margin { left: 16, right: 8, top: 6, bottom: 6 })
                    .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(60) })
                    .show(ui, |ui| {
                        ui.set_min_height(36.0);
                        ui.horizontal_centered(|ui| {
                            ui.label(RichText::new(&text).size(house::BODY).color(Color32::WHITE));
                            ui.add_space(house::L);
                            let light_blue = theme::SHARE;
                            if can_undo
                                && ui
                                    .add(egui::Button::new(RichText::new("Undo").size(house::BODY).strong().color(light_blue)).frame(false))
                                    .clicked()
                            {
                                clicked = true;
                            }
                            let x = egui::Button::new(RichText::new("\u{2715}").size(house::BODY).color(Color32::WHITE)).frame(false);
                            if ui.add(x).on_hover_text("Close").clicked() {
                                closed = true;
                            }
                        });
                    });
            });
        if clicked {
            self.undo(ctx);
        } else if closed {
            self.undo.note = None;
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn note_text(&self) -> Option<&str> {
        self.undo.note.as_ref().map(|n| n.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_step_is_undone_by_a_checked_request() {
        let req = |s: Step| s.undo_request().1;
        let (_, u, b) = (Step::Rotate { id: "p1".into(), left: true }).undo_request();
        assert_eq!((u.as_str(), b.as_deref()), ("api/photos/rotate/p1", Some(&b"{\"dir\":\"right\"}"[..])));
        let (_, u, b) = (Step::Star { id: "f1".into(), on: true }).undo_request();
        assert_eq!((u.as_str(), b), ("api/files/f1/star", Some(b"{\"on\":false}".to_vec())));
        let (_, u, b) = (Step::AlbumAdd { album: "a1".into(), ids: vec!["x".into()] }).undo_request();
        assert_eq!((u.as_str(), b), ("api/photos/albums/a1/remove", Some(b"{\"files\":[\"x\"]}".to_vec())));
        let moved = Step::File { id: "f1".into(), was: "me/A/a.xlsx".into(), now: "me/B/a (2).xlsx".into() };
        assert_eq!(req(moved), "api/files/f1/move?folder=me%2FA&from=me%2FB%2Fa%20(2).xlsx&as=a.xlsx");
        let renamed = Step::File { id: "f1".into(), was: "me/A/a.xlsx".into(), now: "me/A/b.xlsx".into() };
        assert_eq!(req(renamed), "api/files/f1/rename?name=a.xlsx&from=me%2FA%2Fb.xlsx&exact=1");
        let folder = |id: &str, was: &str, now: &str| Step::Folder { id: id.into(), was: was.into(), now: now.into() };
        assert_eq!(req(folder("d7", "me/X", "me/B/X")), "api/folders/move?path=me%2FB%2FX&to=me&as=X&id=d7");
        assert_eq!(req(folder("", "me/X", "me/Y")), "api/folders/rename?path=me%2FY&name=X&exact=1");
        assert_eq!(Step::Made { path: "me/N".into() }.undo_request(), (Method::Delete, "api/folders?path=me%2FN&if_empty=1".into(), None));
        assert_eq!(
            req(Step::Binned { bin: "a.xlsx".into(), original: "me/A/a.xlsx".into() }),
            "api/bin/restore?name=a.xlsx&expect=me%2FA%2Fa.xlsx&exact=1"
        );
    }

    #[test]
    fn an_action_is_kept_once_all_its_items_are_answered() {
        let mut u = Undo::default();
        let g = u.start("Moved to B".into(), 2, true);
        u.answered(g, Some(Step::Folder { id: String::new(), was: "A".into(), now: "B/A".into() }), 0.0);
        assert!(u.done.is_empty() && u.note.is_none(), "one item still on its way");
        u.answered(g, None, 0.0);
        assert_eq!(u.done.len(), 1);
        assert_eq!(u.note.as_ref().map(|n| n.text.as_str()), Some("Moved to B"));
        // A New folder and its naming are one action: undoing it removes the named folder.
        u.made("me/New folder".into());
        let n = u.start_naming();
        u.answered(n, Some(Step::Folder { id: String::new(), was: "me/New folder".into(), now: "me/Plans".into() }), 0.0);
        assert_eq!(u.done.len(), 2);
        assert_eq!(u.done[1].steps, [Step::Made { path: "me/Plans".into() }]);
        // Nothing done (every item failed): nothing to undo.
        let g = u.start("Moved to C".into(), 1, true);
        u.answered(g, None, 0.0);
        assert_eq!(u.done.len(), 2);
    }
}
