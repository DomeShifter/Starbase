//! Live editing (docs/live-editing.md): tabs connect to /api/live with a WebSocket and name a
//! file; every open file has a room, a thread that holds the file in a GridCraft session, applies
//! changes in the order they arrive, sends them to every tab and saves the file a moment after
//! things go quiet.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use gridcraft_engine::Session;
use gridcraft_engine::model::Workbook;
use serde_json::json;
use starsheets_sync::{self as sync, Change, ToClient, ToServer};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::Shared;
use crate::store::now;

/// Quiet time before saving, and the longest a change waits to be saved.
const SAVE_IDLE: Duration = Duration::from_secs(2);
const SAVE_MAX: Duration = Duration::from_secs(30);
/// How often an open file is checked for changes made outside StarSheets (Excel, a file share).
#[cfg(not(test))]
const DISK_CHECK: Duration = Duration::from_secs(3);
#[cfg(test)]
const DISK_CHECK: Duration = Duration::from_millis(200);
/// A room with nobody in it closes after this (once saved).
const EMPTY_FOR: Duration = Duration::from_secs(10);
/// Every this many changes everyone gets a fingerprint of the whole file to check.
const CHECK_EVERY: u64 = 25;
/// Changes kept for undo.
const HISTORY: usize = 200;
/// How often everyone's access to an open file is checked again (also at once after a sharing
/// change, through [`Rooms::recheck_access`], and when the file moves).
#[cfg(not(test))]
const RECHECK: Duration = Duration::from_secs(30);
#[cfg(test)]
const RECHECK: Duration = Duration::from_millis(300);
/// A tab gets the whole file again (Resync) at most this often; asks in between are answered
/// together when the time is up.
#[cfg(not(test))]
const RESYNC_GAP: Duration = Duration::from_secs(2);
#[cfg(test)]
const RESYNC_GAP: Duration = Duration::from_millis(500);

/// Names for connections, made here so no tab can take another's (security review #3).
static NEXT_CONNECTION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

enum RoomMsg {
    Join {
        /// The connection's name, made by the server.
        client: String,
        /// The id the tab chose (Hello `client`): with the username, whose undo history is whose.
        tab: String,
        user: Option<crate::auth::User>,
        can_edit: bool,
        out: UnboundedSender<String>,
        have: Option<String>,
    },
    Leave {
        client: String,
    },
    From {
        client: String,
        msg: ToServer,
    },
    /// The file was replaced through the files API (a version restored): read it again.
    Replaced,
    /// Sharing or accounts changed: check again who may still view or edit.
    Recheck,
    /// Copy every sheet of this spreadsheet file (bytes, name) to the end of the room's file;
    /// answers with the new sheets' names.
    AddSheets {
        name: String,
        bytes: Vec<u8>,
        by: String,
        reply: std::sync::mpsc::Sender<Result<Vec<String>, String>>,
    },
}

#[derive(Default)]
pub struct Rooms {
    map: Mutex<HashMap<String, Sender<RoomMsg>>>,
}

impl Rooms {
    /// Sends to the room for `file`, starting it if needed.
    fn send(&self, app: &Shared, file: &str, msg: RoomMsg) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let msg = match map.get(file) {
            Some(tx) => match tx.send(msg) {
                Ok(()) => return,
                Err(e) => e.0,
            },
            None => msg,
        };
        let (tx, rx) = channel();
        let _ = tx.send(msg);
        map.insert(file.to_string(), tx);
        let (app, file) = (app.clone(), file.to_string());
        let _ = std::thread::Builder::new().name(format!("room-{file}")).spawn(move || Room::run(app, file, rx));
    }

    /// Copies sheets into `file` through its room (starting one if needed), so everyone with it
    /// open sees them at once; waits up to 30 s for the answer.
    pub fn add_sheets(&self, app: &Shared, file: &str, name: String, bytes: Vec<u8>, by: String) -> Result<Vec<String>, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send(app, file, RoomMsg::AddSheets { name, bytes, by, reply: tx });
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "The spreadsheet didn't answer.".to_string())?
    }

    /// The files API replaced this file: an open room reads it again.
    /// How many files are open in rooms right now.
    pub fn open_count(&self) -> usize {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn replaced(&self, file: &str) {
        if let Some(tx) = self.map.lock().unwrap_or_else(|e| e.into_inner()).get(file) {
            let _ = tx.send(RoomMsg::Replaced);
        }
    }

    /// Sharing or accounts changed (a share added, changed or removed, an account removed or
    /// made owner or not): every open file checks again who may still have it open. Someone who
    /// lost access is told and disconnected; someone whose role changed gets the file again as
    /// a viewer or an editor. Cheap: one message per open file; the check runs in each room.
    pub fn recheck_access(&self) {
        for tx in self.map.lock().unwrap_or_else(|e| e.into_inner()).values() {
            let _ = tx.send(RoomMsg::Recheck);
        }
    }
}

// ---------------------------------------------------------------- connections

pub async fn ws(State(app): State<Shared>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    if !crate::signed_in(&app, &headers) {
        return crate::error(401, "please sign in").into_response();
    }
    let user = crate::current_user(&app, &headers);
    // A tab's biggest message is a change sent whole; packed it stays well under MAX_FRAME (and
    // a frame may unpack to at most sync::MAX_UNPACKED).
    upgrade.max_message_size(sync::MAX_FRAME).max_frame_size(sync::MAX_FRAME).on_upgrade(move |socket| connection(app, socket, user))
}

fn text(m: &ToClient) -> String {
    serde_json::to_string(m).unwrap_or_default()
}

/// A person for the log: their username, or "a guest" with sign-in off.
fn who(user: &Option<crate::auth::User>) -> String {
    user.as_ref().map_or_else(|| "a guest".to_string(), |u| u.username.clone())
}

async fn connection(app: Shared, mut socket: WebSocket, user: Option<crate::auth::User>) {
    // The first message says which file.
    let (file, tab, have) = loop {
        match socket.recv().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ToServer>(&t) {
                Ok(ToServer::Hello { file, build, client, protocol, have }) => {
                    // Only a different protocol needs the tab to reload (not every new build).
                    if protocol != sync::PROTOCOL {
                        crate::log::note(&app.store, &format!("tab of {} on an older StarSheets (build {build:?}) asked to reload", who(&user)));
                        let _ = socket.send(Message::Text(text(&ToClient::Reload).into())).await;
                        return;
                    }
                    // Only for undo after reconnecting (with the username); kept short.
                    let tab: String = client.chars().take(64).collect();
                    break (file, tab, have);
                }
                // A tab older than protocol 3 sends `{"t": "hello", …}` without "c": it reads our
                // Reload (`{"t": "reload"}`) the same as before, and gets the new app.
                Err(_) if t.contains("\"hello\"") => {
                    crate::log::note(&app.store, &format!("tab of {} on an older StarSheets asked to reload", who(&user)));
                    let _ = socket.send(Message::Text(text(&ToClient::Reload).into())).await;
                    return;
                }
                _ => return,
            },
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return,
        }
    };
    // Only people the file is shared with get in; viewers can't change it. The role is checked
    // before anything is noted, and the file id (the tab's text) is logged only quoted.
    let role = {
        let (app, user, file) = (app.clone(), user.clone(), file.clone());
        tokio::task::spawn_blocking(move || {
            let _g = app.lock.lock().unwrap_or_else(|e| e.into_inner());
            let role = crate::sharing::file_role(&app.store, app.auth, user.as_ref(), &file);
            if role == crate::sharing::Role::None {
                crate::log::note(&app.store, &format!("{} was refused file {file:?} (no access)", who(&user)));
                return role;
            }
            if let Some(u) = &user {
                let _ = crate::spaces::note_opened(&app.store, u, &file);
            }
            let name = app.store.path_of(&file).ok().flatten().unwrap_or_else(|| format!("{file:?}"));
            crate::log::note(&app.store, &format!("{} opened {name}", who(&user)));
            role
        })
        .await
        .unwrap_or_default()
    };
    if role == crate::sharing::Role::None {
        let _ = socket.send(Message::Text(text(&ToClient::NoAccess { text: NO_ACCESS.into() }).into())).await;
        return;
    }
    let is_doc = {
        let (app, file) = (app.clone(), file.clone());
        tokio::task::spawn_blocking(move || app.store.path_of(&file).ok().flatten().is_some_and(|p| crate::store::is_doc(&p))).await.unwrap_or(false)
    };
    if is_doc {
        let _ = socket.send(Message::Text(text(&ToClient::Message { text: "That file is a document: open it in StarDocs.".into() }).into())).await;
        return;
    }
    let can_edit = role >= crate::sharing::Role::Edit;
    // The connection's name is the server's, never the tab's: a tab can't speak as another.
    let client = format!("c{}", NEXT_CONNECTION.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let (out, mut outbox) = unbounded_channel::<String>();
    app.rooms.send(&app, &file, RoomMsg::Join { client: client.clone(), tab, user, can_edit, out, have });
    // A ping now and then keeps proxies from closing a quiet connection.
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await;
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            },
            incoming = socket.recv() => {
                // Big messages come compressed, as binary frames (sync::pack).
                let t = match incoming {
                    Some(Ok(Message::Text(t))) => Ok(t.to_string()),
                    Some(Ok(Message::Binary(b))) => tokio::task::spawn_blocking(move || sync::unpack(&b)).await.unwrap_or_else(|e| Err(e.to_string())),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => break,
                };
                match t.and_then(|t| serde_json::from_str::<ToServer>(&t).map_err(|e| e.to_string())) {
                    Ok(msg) => app.rooms.send(&app, &file, RoomMsg::From { client: client.clone(), msg }),
                    Err(e) => {
                        let _ = socket.send(Message::Text(text(&ToClient::Message { text: format!("bad message: {e}") }).into())).await;
                    }
                }
            },
            outgoing = outbox.recv() => match outgoing {
                Some(t) => {
                    let m = if t.len() >= sync::PACK_FROM {
                        match tokio::task::spawn_blocking(move || sync::pack(&t).ok_or(t)).await {
                            Ok(Ok(packed)) => Message::Binary(packed.into()),
                            Ok(Err(t)) => Message::Text(t.into()),
                            Err(_) => break,
                        }
                    } else {
                        Message::Text(t.into())
                    };
                    if socket.send(m).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    app.rooms.send(&app, &file, RoomMsg::Leave { client });
}

// ---------------------------------------------------------------- rooms

const NO_ACCESS: &str = "You don't have access to this file.";

struct Client {
    out: UnboundedSender<String>,
    can_edit: bool,
    /// Who it is (None: a guest, with sign-in off), to check their access again.
    user: Option<crate::auth::User>,
    /// Whose undo history this tab uses: the username and the tab's own id, so the same tab
    /// can undo its earlier changes after reconnecting but nobody can undo as someone else.
    undo_as: String,
    /// When this tab was last sent the whole file on asking (Resync), and whether it asked again
    /// since (answered once RESYNC_GAP has passed).
    resync_at: Option<Instant>,
    resync_wanted: bool,
    person: sync::Person,
    /// Where the tab is (its last Presence), sent to people who join later.
    presence: Option<(usize, gridcraft_engine::selection::Selection)>,
    /// Waiting for this change as its finished state (after NeedState); other changes from
    /// this tab are dropped until it comes, and the tab sends them again.
    blocked: Option<u64>,
    /// Changes this tab undid, newest last.
    redo: Vec<Step>,
}

#[derive(Clone)]
struct Step {
    /// Whose change it is (`Client::undo_as`).
    by: String,
    before: Arc<Workbook>,
    after: Arc<Workbook>,
}

struct Room {
    app: Shared,
    file: String,
    session: Session,
    seq: u64,
    clients: HashMap<String, Client>,
    history: VecDeque<Step>,
    /// Who made each recent change, to tell whether a tab's change was made on exactly what
    /// the server had (then its fingerprint must match).
    authors: VecDeque<(u64, String)>,
    dirty_since: Option<Instant>,
    last_change: Instant,
    save_failed_at: Option<Instant>,
    empty_since: Option<Instant>,
    guests: u32,
    /// The file's version on disk when we last read or wrote it, and when we last looked.
    disk: Option<String>,
    disk_checked: Instant,
    /// When the preview picture was last drawn (at most once a minute while people edit).
    thumb_at: Option<Instant>,
    /// Where the file is (its path in the store), to notice it being moved or binned.
    path: Option<String>,
    /// Who made the last change not yet saved (for "Last changed by" in Details).
    last_editor: Option<String>,
    /// The last change made here (username, Unix seconds): kept after saving, for newcomers.
    last_edit: Option<(String, i64)>,
    /// The last "couldn't save" told to everyone, so it isn't repeated every 10 s.
    told: Option<String>,
    /// `seq` right after the file was read: while it's still that, the room's workbook is
    /// exactly what opening the file gives, so a tab that opened that version needn't get it.
    pristine_seq: Option<u64>,
    /// The whole-file fingerprint and the `seq` it is for (worked out once per revision).
    hash: Option<(u64, String)>,
    /// A save running in the background: the `seq` it saves and where its result comes.
    saving: Option<(u64, std::sync::mpsc::Receiver<Saved>)>,
    /// When everyone's access was last checked again.
    access_checked: Instant,
    /// The tab that made the last change and the sheet it is on: the file is saved open on that
    /// sheet, so a download or another device opens where the work was (each tab itself reopens
    /// where its person last was, remembered in the browser).
    last_sheet: Option<(String, usize)>,
}

/// What a background save reports.
struct Saved {
    result: Result<Option<String>, SaveError>,
    /// The editor it noted (given back on failure, for the next try).
    editor: Option<String>,
}

/// Why a save didn't happen.
enum SaveError {
    /// The file changed on disk since we read it: never write over that.
    Outside,
    Failed(String),
}

impl Room {
    fn run(app: Shared, file: String, rx: Receiver<RoomMsg>) {
        let mut room = Room {
            app,
            file,
            session: Session::new(),
            seq: 0,
            clients: HashMap::new(),
            history: VecDeque::new(),
            authors: VecDeque::new(),
            dirty_since: None,
            last_change: Instant::now(),
            save_failed_at: None,
            empty_since: Some(Instant::now()), // until someone joins (a room opened only to add sheets closes too)
            guests: 0,
            disk: None,
            disk_checked: Instant::now(),
            thumb_at: None,
            path: None,
            last_editor: None,
            last_edit: None,
            told: None,
            pristine_seq: None,
            hash: None,
            saving: None,
            access_checked: Instant::now(),
            last_sheet: None,
        };
        let opened = room.open();
        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(msg) => {
                    if let Err(why) = &opened {
                        // The file couldn't be read: say so to whoever joins, and close.
                        if let RoomMsg::Join { out, .. } = msg {
                            let _ = out.send(text(&ToClient::Message { text: why.clone() }));
                        }
                        room.close(&rx);
                        return;
                    }
                    room.handle(msg);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            room.save_finished();
            // Not while our own save writes the file: that would look like a change from outside.
            if room.saving.is_none() && room.disk_checked.elapsed() >= DISK_CHECK {
                room.check_disk();
            }
            if room.access_checked.elapsed() >= RECHECK {
                room.recheck();
            }
            room.serve_resyncs();
            room.save_if_due(false);
            if room.clients.is_empty()
                && room.dirty_since.is_none()
                && room.saving.is_none()
                && room.empty_since.is_some_and(|t| t.elapsed() > EMPTY_FOR)
                && room.close(&rx)
            {
                return;
            }
        }
    }

    /// Takes the room out of the list, unless a message is waiting (someone just joined).
    fn close(&mut self, rx: &Receiver<RoomMsg>) -> bool {
        let mut map = self.app.rooms.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(msg) = rx.try_recv() {
            drop(map);
            self.handle(msg);
            return false;
        }
        map.remove(&self.file);
        true
    }

    fn open(&mut self) -> Result<(), String> {
        let (name, bytes) = {
            let _g = self.app.lock.lock().unwrap_or_else(|e| e.into_inner());
            let path = self
                .app
                .store
                .path_of(&self.file)
                .map_err(|e| format!("Can't read the file list: {e}"))?
                .ok_or("This file isn't on the server any more.")?;
            let full = self.app.store.full(&path);
            self.path = Some(path.clone());
            let bytes = std::fs::read(&full).map_err(|e| format!("Can't read the file: {e}"))?;
            self.disk = self.app.store.version_of(&full).ok();
            self.disk_checked = Instant::now();
            (crate::store::base_of(&path), bytes)
        };
        let mut s = Session::new();
        let b64 = gridcraft_engine::io::base64_encode(&bytes);
        s.execute("file.open", json!({ "name": name, "base64": b64 })).map_err(|e| format!("Can't open the file: {e}"))?;
        // Google Sheets' Clip lives in StarSheets' section of the file: back onto the cells.
        if let Some(d) = s.active_mut() {
            sync::clip_from_host(Arc::make_mut(&mut d.wb));
        }
        self.session = s;
        self.pristine_seq = Some(self.seq);
        Ok(())
    }

    fn wb(&self) -> Option<Arc<Workbook>> {
        self.session.active().map(|d| d.wb.clone())
    }

    fn to(&self, client: &str, m: &ToClient) {
        if let Some(c) = self.clients.get(client) {
            let _ = c.out.send(text(m));
        }
    }

    fn all(&self, m: &ToClient) {
        let t = text(m);
        for c in self.clients.values() {
            let _ = c.out.send(t.clone());
        }
    }

    /// Who changed the file last and when: this room's own last change, else the one noted at
    /// the last save. Shown as the person's display name.
    fn last_edit_now(&self) -> Option<sync::LastEdit> {
        let (username, at) = self.last_edit.clone().or_else(|| crate::spaces::last_change(&self.app.store, &self.file))?;
        let by = crate::auth::users(&self.app.store).into_iter().find(|u| u.username == username).map(|u| u.name).unwrap_or(username);
        Some(sync::LastEdit { by, at })
    }

    /// The whole workbook to send, with what its formulas spill.
    fn snapshot(&self) -> (sync::Whole, Vec<sync::SheetSpill>) {
        match self.wb() {
            Some(wb) => (sync::workbook_json(&wb), sync::workbook_spills(&wb)),
            None => (serde_json::value::to_raw_value(&serde_json::Value::Null).expect("null"), Vec::new()),
        }
    }

    /// The fingerprint of the whole workbook now (worked out once per revision).
    fn whole_hash(&mut self) -> String {
        if let Some((seq, h)) = &self.hash
            && *seq == self.seq
        {
            return h.clone();
        }
        let h = self.wb().map(|wb| sync::fingerprint(&wb)).unwrap_or_default();
        self.hash = Some((self.seq, h.clone()));
        h
    }

    /// Welcome for a tab: without the workbook when the tab already opened exactly this version
    /// and nothing has changed here since (it checks the fingerprint, and asks again if it differs).
    fn welcome(&mut self, have: Option<String>, can_edit: bool, you: &str) -> ToClient {
        let you = you.to_string();
        let last_edit = self.last_edit_now();
        let same = have.is_some() && have == self.disk && self.pristine_seq == Some(self.seq) && self.dirty_since.is_none();
        if same {
            crate::log::note(
                &self.app.store,
                &format!("{}: the tab already had this version (sent a check, not the file)", self.path.clone().unwrap_or_default()),
            );
            return ToClient::Welcome { seq: self.seq, workbook: None, spills: Vec::new(), hash: Some(self.whole_hash()), can_edit, last_edit, you };
        }
        let (workbook, spills) = self.snapshot();
        ToClient::Welcome { seq: self.seq, workbook: Some(workbook), spills, hash: None, can_edit, last_edit, you }
    }

    fn handle(&mut self, msg: RoomMsg) {
        match msg {
            RoomMsg::Join { client, tab, user, can_edit, out, have } => {
                // Names are the server's and never reused, so this doesn't happen; if it did, the
                // one already here stays (nobody can push someone else out).
                if self.clients.contains_key(&client) {
                    let _ = out.send(text(&ToClient::Message { text: "This connection is already open.".into() }));
                    return;
                }
                let _ = out.send(text(&self.welcome(have, can_edit, &client)));
                let undo_as = format!("{}/{tab}", user.as_ref().map_or("", |u| u.username.as_str()));
                // Without sign-in (STARSHEETS_AUTH=off) people are guests, numbered as they come.
                let person = match user.clone() {
                    Some(u) => sync::Person { client: client.clone(), username: u.username, name: u.name, colour: u.colour },
                    None => {
                        self.guests += 1;
                        let colour = crate::auth::COLOURS[(self.guests as usize - 1) % crate::auth::COLOURS.len()].to_string();
                        sync::Person { client: client.clone(), username: String::new(), name: format!("Guest {}", self.guests), colour }
                    }
                };
                for (other, c) in &self.clients {
                    if let Some((sheet, sel)) = &c.presence {
                        let _ = out.send(text(&ToClient::Presence { client: other.clone(), sheet: *sheet, sel: sel.clone() }));
                    }
                }
                self.clients.insert(
                    client,
                    Client {
                        out,
                        person,
                        can_edit,
                        user,
                        undo_as,
                        resync_at: None,
                        resync_wanted: false,
                        presence: None,
                        blocked: None,
                        redo: Vec::new(),
                    },
                );
                self.empty_since = None;
                self.people();
            }
            RoomMsg::Leave { client } => {
                if let Some(c) = self.clients.get(&client) {
                    let name = if c.person.username.is_empty() { c.person.name.clone() } else { c.person.username.clone() };
                    crate::log::note(&self.app.store, &format!("{name} closed {}", self.path.clone().unwrap_or_default()));
                }
                self.clients.remove(&client);
                self.people();
                if self.clients.is_empty() {
                    self.empty_since = Some(Instant::now());
                    self.save_if_due(true);
                }
            }
            RoomMsg::Replaced => self.reload(),
            RoomMsg::Recheck => self.recheck(),
            RoomMsg::AddSheets { name, bytes, by, reply } => {
                let _ = reply.send(self.add_sheets(&name, &bytes, &by));
            }
            // From a connection that isn't (or is no longer) in the room: nothing it says counts.
            RoomMsg::From { client, .. } if !self.clients.contains_key(&client) => {}
            RoomMsg::From { client, msg }
                if matches!(msg, ToServer::Change { .. } | ToServer::Undo | ToServer::Redo)
                    && !self.clients.get(&client).is_some_and(|c| c.can_edit) =>
            {
                if let ToServer::Change { cseq, .. } = msg {
                    self.to(&client, &ToClient::Rejected { cseq, why: gridcraft_engine::VIEW_ONLY.into() });
                }
            }
            RoomMsg::From { client, msg } => match msg {
                ToServer::Change { cseq, base, change } => self.change(&client, cseq, base, change),
                ToServer::Undo => self.undo(&client),
                ToServer::Redo => self.redo(&client),
                ToServer::Resync => {
                    if let Some(c) = self.clients.get_mut(&client) {
                        c.resync_wanted = true;
                    }
                    self.serve_resyncs();
                }
                ToServer::Presence { sheet, sel } => {
                    if let Some(c) = self.clients.get_mut(&client) {
                        c.presence = Some((sheet, sel.clone()));
                    }
                    if let Some((by, at)) = &mut self.last_sheet
                        && *by == client
                    {
                        *at = sheet;
                    }
                    let m = text(&ToClient::Presence { client: client.clone(), sheet, sel });
                    for (other, c) in &self.clients {
                        if *other != client {
                            let _ = c.out.send(m.clone());
                        }
                    }
                }
                ToServer::Hello { .. } => {}
            },
        }
    }

    /// Tells everyone who has the file open (each tab learns which entry is its own).
    fn people(&self) {
        let mut people: Vec<sync::Person> = self.clients.values().map(|c| c.person.clone()).collect();
        people.sort_by(|a, b| a.name.cmp(&b.name).then(a.client.cmp(&b.client)));
        for (id, c) in &self.clients {
            let _ = c.out.send(text(&ToClient::People { people: people.clone(), you: id.clone() }));
        }
    }

    /// Whether every change after `base` came from `client`, so its change was made on exactly
    /// what the server has.
    fn same_base(&self, client: &str, base: u64) -> bool {
        base == self.seq
            || self.authors.iter().filter(|(s, _)| *s > base).all(|(_, c)| c == client) && self.authors.front().is_some_and(|(s, _)| *s <= base + 1)
    }

    fn change(&mut self, client: &str, cseq: u64, base: u64, change: Change) {
        let Some(c) = self.clients.get(client) else { return };
        if let Some(waiting) = c.blocked
            && (cseq != waiting || !matches!(change, Change::State { .. }))
        {
            return;
        }
        if let Change::Cmd { id, .. } = &change
            && !sync::server_allows(id)
        {
            self.to(client, &ToClient::Rejected { cseq, why: format!("The server doesn't run \"{id}\" for a tab.") });
            return;
        }
        let Some(before) = self.wb() else { return };
        if let Err(why) = sync::apply(&mut self.session, &change) {
            sync::restore_workbook(&mut self.session, &before);
            self.to(client, &ToClient::Rejected { cseq, why });
            return;
        }
        let Some(after) = self.wb() else { return };
        if let Change::Cmd { fp, .. } = &change
            && self.same_base(client, base)
            && sync::change_fingerprint(&before, &after) != *fp
        {
            sync::restore_workbook(&mut self.session, &before);
            if let Some(c) = self.clients.get_mut(client) {
                c.blocked = Some(cseq);
            }
            self.to(client, &ToClient::NeedState { cseq });
            return;
        }
        // Where the tab is: its last Presence, else the sheet the change was made on.
        let sheet = match &change {
            Change::Cmd { sheet, .. } => Some(*sheet),
            Change::State { .. } => None,
        };
        if let Some(at) = self.clients.get(client).and_then(|c| c.presence.as_ref().map(|p| p.0)).or(sheet) {
            self.last_sheet = Some((client.to_string(), at));
        }
        if let Some(c) = self.clients.get_mut(client) {
            c.blocked = None;
            c.redo.clear();
            if !c.person.username.is_empty() {
                self.last_editor = Some(c.person.username.clone());
                self.last_edit = Some((c.person.username.clone(), crate::store::now() as i64));
            }
        }
        self.record(client, before, after);
        self.all(&ToClient::Applied { seq: self.seq, client: client.to_string(), cseq, change });
        if self.seq.is_multiple_of(CHECK_EVERY) {
            self.check();
        }
    }

    fn record(&mut self, client: &str, before: Arc<Workbook>, after: Arc<Workbook>) {
        self.seq += 1;
        let by = self.clients.get(client).map(|c| c.undo_as.clone()).unwrap_or_default();
        self.history.push_back(Step { by, before, after });
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.authors.push_back((self.seq, client.to_string()));
        if self.authors.len() > 1000 {
            self.authors.pop_front();
        }
        self.dirty_since.get_or_insert_with(Instant::now);
        self.last_change = Instant::now();
    }

    fn check(&mut self) {
        if self.wb().is_some() {
            let hash = self.whole_hash();
            self.all(&ToClient::Check { seq: self.seq, hash });
        }
    }

    /// Undo of your own latest change. When it's also the file's latest change the whole file goes
    /// back; otherwise the cells it changed go back, as long as it changed only cells and nobody
    /// has changed those cells since (`sync::undo_cells`).
    fn undo(&mut self, client: &str) {
        let Some(me) = self.clients.get(client).map(|c| c.undo_as.clone()) else { return };
        let Some(k) = self.history.iter().rposition(|s| s.by == me) else {
            self.to(client, &ToClient::Message { text: "Nothing to undo.".into() });
            return;
        };
        let Some(current) = self.wb() else { return };
        let step = self.history[k].clone();
        if k + 1 == self.history.len() && Arc::ptr_eq(&current, &step.after) {
            sync::restore_workbook(&mut self.session, &step.before);
        } else {
            match sync::undo_cells(&step.before, &step.after, &current) {
                Ok(wb) => sync::set_workbook_recalc(&mut self.session, wb),
                Err(why) => {
                    self.to(client, &ToClient::Message { text: why.into() });
                    return;
                }
            }
        }
        self.history.remove(k);
        if let Some(c) = self.clients.get_mut(client) {
            c.redo.push(step);
        }
        self.server_change(current);
    }

    /// Redo of what you undid, the same way round.
    fn redo(&mut self, client: &str) {
        let Some(step) = self.clients.get(client).and_then(|c| c.redo.last()).cloned() else {
            self.to(client, &ToClient::Message { text: "Nothing to redo.".into() });
            return;
        };
        let Some(current) = self.wb() else { return };
        if Arc::ptr_eq(&current, &step.before) {
            sync::restore_workbook(&mut self.session, &step.after);
        } else {
            match sync::undo_cells(&step.after, &step.before, &current) {
                Ok(wb) => sync::set_workbook_recalc(&mut self.session, wb),
                Err(why) => {
                    self.to(client, &ToClient::Message { text: why.replace("undo", "redo") });
                    return;
                }
            }
        }
        if let Some(c) = self.clients.get_mut(client) {
            c.redo.pop();
        }
        self.history.push_back(step);
        self.server_change(current);
    }

    /// A change the server made (undo, redo): everyone applies it, as the finished state of what
    /// it changed (sender "" = the server).
    fn server_change(&mut self, before: Arc<Workbook>) {
        let Some(after) = self.wb() else { return };
        self.seq += 1;
        self.authors.push_back((self.seq, String::new()));
        self.dirty_since.get_or_insert_with(Instant::now);
        self.last_change = Instant::now();
        let patch = sync::make_patch(&before, &after);
        self.all(&ToClient::Applied { seq: self.seq, client: String::new(), cseq: 0, change: Change::State { patch } });
    }

    /// Sends the whole file to tabs that asked (Resync), each at most once per RESYNC_GAP: a tab
    /// asking over and over can't keep the room busy making copies of a big file.
    fn serve_resyncs(&mut self) {
        let due: Vec<String> = self
            .clients
            .iter()
            .filter(|(_, c)| c.resync_wanted && c.resync_at.is_none_or(|t| t.elapsed() >= RESYNC_GAP))
            .map(|(k, _)| k.clone())
            .collect();
        if due.is_empty() {
            return;
        }
        let (workbook, spills) = self.snapshot();
        let m = text(&ToClient::Reset { seq: self.seq, workbook, spills });
        for k in due {
            if let Some(c) = self.clients.get_mut(&k) {
                c.resync_wanted = false;
                c.resync_at = Some(Instant::now());
                let _ = c.out.send(m.clone());
            }
        }
    }

    /// Checks again what everyone here may do, the same way joining does (security review
    /// #10). Someone who may no longer open the file is told and disconnected; someone whose
    /// role changed between viewing and editing gets the file again with the new role.
    fn recheck(&mut self) {
        self.access_checked = Instant::now();
        if self.clients.is_empty() {
            return;
        }
        let roles: Vec<(String, crate::sharing::Role)> = {
            let _g = self.app.lock.lock().unwrap_or_else(|e| e.into_inner());
            // Accounts as they are now: removed ones are gone, owners may have changed.
            let users = crate::auth::users(&self.app.store);
            let mut seen: HashMap<Option<String>, crate::sharing::Role> = HashMap::new();
            self.clients
                .iter()
                .map(|(k, c)| {
                    let name = c.user.as_ref().map(|u| u.username.clone());
                    let role = *seen.entry(name.clone()).or_insert_with(|| {
                        let now = name.as_ref().and_then(|n| users.iter().find(|u| &u.username == n));
                        // Someone signed in whose account is gone has no access (unlike a guest).
                        if name.is_some() && now.is_none() {
                            return crate::sharing::Role::None;
                        }
                        crate::sharing::file_role(&self.app.store, self.app.auth, now, &self.file)
                    });
                    (k.clone(), role)
                })
                .collect()
        };
        let mut left = false;
        for (k, role) in roles {
            if role == crate::sharing::Role::None {
                if let Some(c) = self.clients.remove(&k) {
                    crate::log::note(
                        &self.app.store,
                        &format!("{} no longer has access to {}: disconnected", who(&c.user), self.path.clone().unwrap_or_default()),
                    );
                    // Dropping `out` ends the connection once this is sent.
                    let _ = c.out.send(text(&ToClient::NoAccess { text: NO_ACCESS.into() }));
                }
                left = true;
                continue;
            }
            let can_edit = role >= crate::sharing::Role::Edit;
            if self.clients.get(&k).is_some_and(|c| c.can_edit != can_edit) {
                let welcome = self.welcome(None, can_edit, &k);
                if let Some(c) = self.clients.get_mut(&k) {
                    c.can_edit = can_edit;
                    c.blocked = None;
                    c.redo.clear();
                    let _ = c.out.send(text(&welcome));
                }
            }
        }
        if left {
            self.people();
            if self.clients.is_empty() {
                self.empty_since = Some(Instant::now());
                self.save_if_due(true);
            }
        }
    }

    /// Starts a save once changes have rested (or have waited too long). The work (writing the
    /// .xlsx, the preview picture) runs on its own thread, so the room keeps taking changes and
    /// the server's file lock is held only while the file is written.
    fn save_if_due(&mut self, now_please: bool) {
        let Some(since) = self.dirty_since else { return };
        if self.saving.is_some() || self.save_failed_at.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
            return;
        }
        if !now_please && self.last_change.elapsed() < SAVE_IDLE && since.elapsed() < SAVE_MAX {
            return;
        }
        let Some(wb) = self.wb() else { return };
        let thumb = self.clients.is_empty() || self.thumb_at.is_none_or(|t| t.elapsed() > Duration::from_secs(60));
        if thumb {
            self.thumb_at = Some(Instant::now());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (app, file, disk, editor) = (self.app.clone(), self.file.clone(), self.disk.clone(), self.last_editor.take());
        let sheet = self.last_sheet.as_ref().map(|(_, at)| *at);
        std::thread::spawn(move || {
            let result = save_file(&app, &file, &wb, sheet, disk.as_deref(), editor.as_deref());
            if let Ok(Some(v)) = &result
                && thumb
                && let Ok(png) = crate::thumbs::draw(&wb)
            {
                let _ = crate::thumbs::keep(&app.store, &file, v, &png);
            }
            let _ = tx.send(Saved { result, editor });
        });
        self.saving = Some((self.seq, rx));
    }

    /// Takes in a background save's result, if it has finished.
    fn save_finished(&mut self) {
        let Some((seq, rx)) = &self.saving else { return };
        let saved = match rx.try_recv() {
            Ok(s) => s,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Saved { result: Err(SaveError::Failed("the save stopped".into())), editor: None },
        };
        let seq = *seq;
        self.saving = None;
        match saved.result {
            Ok(version) => {
                self.disk = version;
                // Changes made while it saved are still to be saved.
                if self.seq == seq {
                    self.dirty_since = None;
                }
                self.save_failed_at = None;
                self.told = None;
                self.all(&ToClient::Saved { at: now() as i64 });
            }
            Err(SaveError::Outside) => {
                self.last_editor = self.last_editor.take().or(saved.editor);
                self.check_disk();
            }
            Err(SaveError::Failed(why)) => {
                self.last_editor = self.last_editor.take().or(saved.editor);
                self.save_failed_at = Some(Instant::now());
                if self.told.as_deref() != Some(why.as_str()) {
                    crate::log::note(&self.app.store, &format!("couldn't save {}: {why}", self.path.clone().unwrap_or_default()));
                }
                // Moved to the bin: check_disk says so once, in plain words; other failures are
                // told once each, not every 10 s.
                if self.disk_now().is_some() && self.told.as_deref() != Some(why.as_str()) {
                    self.all(&ToClient::Message { text: format!("Couldn't save: {why}") });
                    self.told = Some(why);
                }
            }
        }
    }

    /// The file's version on disk now (None: gone, moved to the bin, or unreadable).
    fn disk_now(&self) -> Option<String> {
        let _g = self.app.lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.app.store.path_of(&self.file).ok()??;
        self.app.store.version_of(&self.app.store.full(&path)).ok()
    }

    /// Someone may have changed the file outside StarSheets (in Excel, over a file share). If so,
    /// their version wins the file: changes made here that weren't saved yet go into a copy next
    /// to it, and every tab is shown the file as it is on disk now. Nothing is written over.
    fn check_disk(&mut self) {
        self.disk_checked = Instant::now();
        self.check_place();
        let now_on_disk = self.disk_now();
        if now_on_disk.is_none() || now_on_disk == self.disk {
            return;
        }
        let kept = if self.dirty_since.is_some() { self.wb().map(|wb| self.keep_copy(&wb)) } else { None };
        crate::log::note(
            &self.app.store,
            &format!(
                "{} changed outside StarSheets; {}",
                self.path.clone().unwrap_or_default(),
                match &kept {
                    Some(Ok(n)) => format!("unsaved changes kept in {n}"),
                    Some(Err(e)) => format!("unsaved changes could NOT be kept: {e}"),
                    None => "nothing was unsaved".into(),
                }
            ),
        );
        if matches!(kept, Some(Ok(_))) {
            self.dirty_since = None; // kept: don't make a second copy if reading fails below
        }
        self.reload();
        let text = match kept {
            Some(Ok(name)) => format!(
                "This file was changed outside StarSheets (for example in Excel). You're now seeing that version; the changes made here in the last few seconds are kept in \u{201c}{name}\u{201d} in the same folder."
            ),
            Some(Err(why)) => {
                format!("This file was changed outside StarSheets, and the last few seconds of changes made here couldn't be kept: {why}")
            }
            None => "This file was changed outside StarSheets (for example in Excel), so it was opened again.".to_string(),
        };
        self.all(&ToClient::Message { text });
    }

    /// Notices the file being renamed, moved (here or in another tab, by anyone) or put in the bin,
    /// and tells every tab: they show the new name and place; in the bin, nothing saves until
    /// it's restored (then saving carries on by itself).
    fn check_place(&mut self) {
        let now = {
            let _g = self.app.lock.lock().unwrap_or_else(|e| e.into_inner());
            self.app.store.path_of(&self.file).ok().flatten()
        };
        if now == self.path {
            return;
        }
        let (name, folder) = match (&now, &self.path) {
            (Some(p), _) | (None, Some(p)) => (crate::store::base_of(p), crate::store::parent_of(p)),
            (None, None) => return,
        };
        crate::log::note(
            &self.app.store,
            &format!("{} is now {}", self.path.clone().unwrap_or_default(), now.clone().unwrap_or_else(|| "in the bin".into())),
        );
        self.all(&ToClient::Moved { name, folder, binned: now.is_none() });
        self.path = now;
        // Moved to another folder or space: other shares may apply now.
        self.recheck();
    }

    /// Copies the sheets of another spreadsheet file to the end of this one, as a change made by
    /// the server (every tab gets it at once; undo history isn't touched).
    fn add_sheets(&mut self, name: &str, bytes: &[u8], by: &str) -> Result<Vec<String>, String> {
        let mut s = Session::new();
        s.execute("file.open", json!({ "name": name, "base64": gridcraft_engine::io::base64_encode(bytes) }))
            .map_err(|e| format!("Can't read that sheet: {e}"))?;
        let src = s.active().map(|d| d.wb.clone()).ok_or("nothing to copy")?;
        let before = self.wb().ok_or("the spreadsheet isn't open")?;
        let mut wb = (*before).clone();
        let mut names = Vec::new();
        for i in 0..src.sheets.len() {
            if let Some(n) = sync::copy_sheet_into(&src, i, &mut wb) {
                names.push(n);
            }
        }
        sync::set_workbook_recalc(&mut self.session, wb);
        self.server_change(before);
        if !by.is_empty() {
            self.last_editor = Some(by.to_string());
            self.last_edit = Some((by.to_string(), crate::store::now() as i64));
        }
        Ok(names)
    }

    /// Saves the room's workbook as a new file next to this one; returns its name.
    fn keep_copy(&self, wb: &Workbook) -> Result<String, String> {
        let _g = self.app.lock.lock().unwrap_or_else(|e| e.into_inner());
        let store = &self.app.store;
        let path = store.path_of(&self.file).map_err(|e| e.to_string())?.ok_or("the file is gone")?;
        let folder = crate::store::parent_of(&path);
        let (stem, _) = crate::store::split_name(&crate::store::base_of(&path));
        let name =
            store.claim_name(&folder, &format!("{stem} (StarSheets copy {})", crate::store::readable_stamp()), ".xlsx", None).map_err(|e| e.msg)?;
        let bytes = gridcraft_engine::io::save_bytes(wb, &name).map_err(|e| e.to_string())?;
        store.write_file(&store.full(&crate::store::join(&folder, &name)), &bytes).map_err(|e| e.to_string())?;
        Ok(name)
    }

    /// Reads the file again and gives every tab the new state (history starts again).
    fn reload(&mut self) {
        if self.open().is_ok() {
            self.history.clear();
            self.dirty_since = None;
            self.seq += 1;
            self.pristine_seq = Some(self.seq);
            let (workbook, spills) = self.snapshot();
            self.all(&ToClient::Reset { seq: self.seq, workbook, spills });
        }
    }
}

/// Writes `wb` over file `id`, as long as the file on disk is still `expected` (never over a
/// change made outside StarSheets). The .xlsx is made before taking the server's file lock.
/// Returns the file's new version. `sheet`: the sheet the file opens on (the last editor's).
fn save_file(
    app: &Shared,
    id: &str,
    wb: &Workbook,
    sheet: Option<usize>,
    expected: Option<&str>,
    editor: Option<&str>,
) -> Result<Option<String>, SaveError> {
    let failed = |e: String| SaveError::Failed(e);
    // Cells with Google Sheets' Clip are listed in StarSheets' section (Excel has no such setting).
    let mut wb = wb.clone();
    if let Some(at) = sheet.filter(|at| wb.sheet(*at).is_some_and(|sh| sh.visibility == gridcraft_engine::model::Visibility::Visible)) {
        wb.set_active_sheet(at);
    }
    sync::clip_into_host(&mut wb);
    let name = app.store.path_of(id).ok().flatten().map(|p| crate::store::base_of(&p)).unwrap_or_else(|| "file.xlsx".into());
    let bytes = gridcraft_engine::io::save_bytes(&wb, &name).map_err(|e| failed(e.to_string()))?;
    let _g = app.lock.lock().unwrap_or_else(|e| e.into_inner());
    let path = app.store.path_of(id).map_err(|e| failed(e.to_string()))?.ok_or_else(|| failed("the file was moved to the bin".into()))?;
    let full = app.store.full(&path);
    if app.store.version_of(&full).ok().as_deref() != expected {
        return Err(SaveError::Outside);
    }
    app.store.keep_version(id, &full).map_err(|e| failed(e.to_string()))?;
    app.store.write_file(&full, &bytes).map_err(|e| failed(e.to_string()))?;
    let version = app.store.version_of(&full).ok();
    if let Some(who) = editor {
        let _ = crate::spaces::note_changed(&app.store, id, who);
    }
    crate::log::note(&app.store, &format!("saved {path} (last change by {})", editor.unwrap_or("someone")));
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;
    use crate::store::Store;
    use std::collections::HashMap;
    use tokio::sync::mpsc::UnboundedReceiver;

    struct Tab {
        name: String,
        rx: UnboundedReceiver<String>,
        session: Session,
        seq: u64,
        cseq: u64,
        /// Who-is-here messages, kept aside.
        seen: Vec<ToClient>,
    }

    fn app() -> (Shared, String, std::path::PathBuf) {
        app_with(false)
    }

    /// A test server with one file; `auth`: sign-in on (people and sharing apply).
    fn app_with(auth: bool) -> (Shared, String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("starsheets-live-{}", crate::store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.csv"), "a,b\n1,2\n").unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: false };
        let id = store.synced_index().unwrap().keys().next().unwrap().clone();
        let app =
            Arc::new(App { store, base: dir.clone(), lock: Mutex::new(()), location: String::new(), auth, site: None, rooms: Rooms::default() });
        (app, id, dir)
    }

    fn next(t: &mut Tab) -> ToClient {
        for _ in 0..400 {
            if let Ok(m) = t.rx.try_recv() {
                let m: ToClient = serde_json::from_str(&m).unwrap();
                if matches!(m, ToClient::People { .. } | ToClient::Presence { .. }) {
                    t.seen.push(m);
                    continue;
                }
                return m;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("{}: nothing from the server", t.name);
    }

    fn join(app: &Shared, file: &str, name: &str) -> Tab {
        join_as(app, file, name, name, None, true)
    }

    /// Joins as connection `name` (the server's name for it), from a tab with id `tab`.
    fn join_as(app: &Shared, file: &str, name: &str, tab: &str, user: Option<crate::auth::User>, can_edit: bool) -> Tab {
        let (out, rx) = unbounded_channel();
        app.rooms.send(app, file, RoomMsg::Join { client: name.into(), tab: tab.into(), user, can_edit, out, have: None });
        let mut t = Tab { name: name.into(), rx, session: Session::new(), seq: 0, cseq: 0, seen: Vec::new() };
        match next(&mut t) {
            ToClient::Welcome { seq, workbook, .. } => {
                t.session.execute("file.new", json!({})).unwrap();
                sync::set_workbook(&mut t.session, sync::workbook_from_json(&workbook.expect("a test tab gets the workbook"), None).unwrap());
                t.seq = seq;
            }
            m => panic!("expected welcome, got {m:?}"),
        }
        t.session.sync_log = Some(Vec::new());
        t
    }

    /// Runs a command in the tab and sends what it logged, as the web app does.
    fn edit(app: &Shared, file: &str, t: &mut Tab, id: &str, p: serde_json::Value) {
        t.session.execute(id, p).unwrap();
        for st in t.session.sync_log.replace(Vec::new()).unwrap() {
            let change = if sync::replayable(&st.id) && sync::server_allows(&st.id) {
                let fp = sync::change_fingerprint(&st.before, &st.after);
                Change::Cmd { sheet: st.sheet, sel: st.selection, id: st.id, params: st.params, locale: sync::InputLocale::current(), fp }
            } else {
                Change::State { patch: sync::make_patch(&st.before, &st.after) }
            };
            t.cseq += 1;
            app.rooms.send(app, file, RoomMsg::From { client: t.name.clone(), msg: ToServer::Change { cseq: t.cseq, base: t.seq, change } });
        }
    }

    /// Takes one Applied: the sender just notes the revision, others apply the change.
    fn take_applied(t: &mut Tab) -> (String, u64) {
        match next(t) {
            ToClient::Applied { seq, client, cseq, change } => {
                if client != t.name {
                    sync::apply(&mut t.session, &change).unwrap();
                }
                t.seq = seq;
                (client, cseq)
            }
            m => panic!("{}: expected applied, got {m:?}", t.name),
        }
    }

    fn print(t: &Tab) -> String {
        sync::fingerprint(&t.session.active().unwrap().wb)
    }

    #[test]
    fn two_tabs_stay_identical_and_the_file_is_saved() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        let mut b = join(&app, &file, "b");
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "C1", "input": "from a"}));
        edit(&app, &file, &mut b, "cell.set", json!({"cell": "C2", "input": "from b"}));
        for _ in 0..2 {
            take_applied(&mut a);
            take_applied(&mut b);
        }
        assert_eq!(print(&a), print(&b));
        // A paste travels as finished state.
        a.session.sync_log = None;
        a.session.execute("selection.set", json!({"range": "A1:B2"})).unwrap();
        a.session.execute("edit.copy", json!({})).unwrap();
        a.session.execute("selection.set", json!({"range": "E5"})).unwrap();
        a.session.sync_log = Some(Vec::new());
        edit(&app, &file, &mut a, "edit.paste", json!({}));
        take_applied(&mut a);
        take_applied(&mut b);
        assert_eq!(print(&a), print(&b));
        // Leaving saves at once.
        app.rooms.send(&app, &file, RoomMsg::Leave { client: "a".into() });
        app.rooms.send(&app, &file, RoomMsg::Leave { client: "b".into() });
        std::thread::sleep(Duration::from_millis(400));
        let saved = std::fs::read_to_string(dir.join("t.csv")).unwrap();
        assert!(saved.contains("from a") && saved.contains("from b"), "{saved}");
        assert_eq!(std::fs::read_dir(dir.join(".starsheets/versions").join(&file)).unwrap().count(), 1, "the old copy is kept");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A tab that already opened the file's current version gets a fingerprint instead of the
    /// whole workbook (protocol 5); a tab with another version, or once the file has changed
    /// here, gets the workbook.
    #[test]
    fn a_tab_with_the_same_version_is_not_sent_the_workbook_again() {
        let (app, file, dir) = app();
        let version = app.store.version_of(&dir.join("t.csv")).unwrap();
        let welcome = |have: Option<String>, name: &str| {
            let (out, rx) = unbounded_channel();
            app.rooms.send(&app, &file, RoomMsg::Join { client: name.into(), tab: name.into(), user: None, can_edit: true, out, have });
            let mut t = Tab { name: name.into(), rx, session: Session::new(), seq: 0, cseq: 0, seen: Vec::new() };
            next(&mut t)
        };
        let ToClient::Welcome { workbook: None, hash: Some(hash), .. } = welcome(Some(version.clone()), "same") else {
            panic!("expected no workbook")
        };
        // The hash is the file as anyone opening it gets it.
        let mut s = Session::new();
        let b64 = gridcraft_engine::io::base64_encode(&std::fs::read(dir.join("t.csv")).unwrap());
        s.execute("file.open", json!({ "name": "t.csv", "base64": b64 })).unwrap();
        sync::clip_from_host(Arc::make_mut(&mut s.active_mut().unwrap().wb));
        assert_eq!(hash, sync::fingerprint(&s.active().unwrap().wb));
        assert!(matches!(welcome(Some("other".into()), "old"), ToClient::Welcome { workbook: Some(_), .. }));
        assert!(matches!(welcome(None, "none"), ToClient::Welcome { workbook: Some(_), .. }));
        let mut a = join(&app, &file, "a");
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "C1", "input": "x"}));
        take_applied(&mut a);
        assert!(matches!(welcome(Some(version), "late"), ToClient::Welcome { workbook: Some(_), .. }), "changed since: sent whole");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Waits for the next Message, skipping anything else.
    fn message(t: &mut Tab) -> String {
        for _ in 0..20 {
            if let ToClient::Message { text } = next(t) {
                return text;
            }
        }
        panic!("no message");
    }

    #[test]
    fn open_tabs_hear_about_moves_and_get_copied_sheets() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        // Renamed (by the files API, or anyone): the tab is told the new name and folder.
        std::fs::create_dir_all(dir.join("Work")).unwrap();
        {
            let _g = app.lock.lock().unwrap();
            let q: HashMap<String, String> = [("folder".to_string(), "Work".to_string())].into();
            app.store.route("POST", &["files".into(), file.clone(), "move".into()], &q, None, b"").unwrap();
        }
        let mut moved = None;
        for _ in 0..20 {
            if let ToClient::Moved { name, folder, binned } = next(&mut a) {
                moved = Some((name, folder, binned));
                break;
            }
        }
        assert_eq!(moved, Some(("t.csv".into(), "Work".into(), false)));
        // Another spreadsheet's sheet copied in: every tab gets it.
        let names = app.rooms.add_sheets(&app, &file, "x.csv".into(), b"from,another\n1,2\n".to_vec(), String::new()).unwrap();
        assert_eq!(names.len(), 1);
        assert!(matches!(next(&mut a), ToClient::Applied { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn changes_made_outside_starsheets_are_never_overwritten() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        // Changed outside while nothing is unsaved: the room opens the file again.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("t.csv"), "a,b\n1,2\nexcel,3\n").unwrap();
        assert!(matches!(next(&mut a), ToClient::Reset { .. }));
        assert!(message(&mut a).contains("opened again"));
        // Changed outside while a change waits to be saved: ours goes into a copy, theirs stays.
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "C1", "input": "ours"}));
        take_applied(&mut a);
        std::fs::write(dir.join("t.csv"), "a,b\ntheirs,9\n").unwrap();
        assert!(matches!(next(&mut a), ToClient::Reset { .. }));
        assert!(message(&mut a).contains("StarSheets copy"));
        app.rooms.send(&app, &file, RoomMsg::Leave { client: "a".into() });
        std::thread::sleep(Duration::from_millis(400));
        assert!(std::fs::read_to_string(dir.join("t.csv")).unwrap().contains("theirs"), "their version is untouched");
        let copies: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("StarSheets copy"))
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_wrong_replay_asks_for_the_finished_state() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        let change = Change::Cmd {
            sheet: 0,
            sel: gridcraft_engine::selection::Selection::default(),
            id: "cell.set".into(),
            params: json!({"cell": "A1", "input": "x"}),
            locale: sync::InputLocale::current(),
            fp: "not the right one".into(),
        };
        app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Change { cseq: 1, base: 0, change } });
        assert!(matches!(next(&mut a), ToClient::NeedState { cseq: 1 }));
        // Until the state comes, its next change is dropped; the state is accepted.
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "B9", "input": "later"}));
        let wb = a.session.active().unwrap().wb.clone();
        let patch = sync::make_patch(&wb, &wb);
        app.rooms.send(
            &app,
            &file,
            RoomMsg::From { client: "a".into(), msg: ToServer::Change { cseq: 1, base: 0, change: Change::State { patch } } },
        );
        assert_eq!(take_applied(&mut a), ("a".into(), 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn people_and_where_they_are() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        let sel = gridcraft_engine::selection::Selection::default();
        app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Presence { sheet: 0, sel: sel.clone() } });
        let mut b = join(&app, &file, "b");
        // b hears where a already is, and both hear that two people are here.
        std::thread::sleep(Duration::from_millis(100));
        while let Ok(m) = b.rx.try_recv() {
            b.seen.push(serde_json::from_str(&m).unwrap());
        }
        assert!(b.seen.iter().any(|m| matches!(m, ToClient::Presence { client, .. } if client == "a")), "{:?}", b.seen.len());
        assert!(b.seen.iter().any(|m| matches!(m, ToClient::People { people, you } if people.len() == 2 && you == "b")));
        while let Ok(m) = a.rx.try_recv() {
            a.seen.push(serde_json::from_str(&m).unwrap());
        }
        assert!(a.seen.iter().any(|m| matches!(m, ToClient::People { people, .. } if people.iter().any(|p| p.name == "Guest 2"))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn undo_only_your_own_latest_change() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        let mut b = join(&app, &file, "b");
        let start = print(&a);
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "A5", "input": "a's"}));
        take_applied(&mut a);
        take_applied(&mut b);
        app.rooms.send(&app, &file, RoomMsg::From { client: "b".into(), msg: ToServer::Undo });
        assert!(matches!(next(&mut b), ToClient::Message { .. }));
        app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Undo });
        for t in [&mut a, &mut b] {
            assert_eq!(take_applied(t).0, "", "undo comes from the server");
        }
        assert_eq!(print(&a), start);
        assert_eq!(print(&b), start);
        // Undo of a's change after b changed another cell keeps b's change.
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "A7", "input": "a again"}));
        take_applied(&mut a);
        take_applied(&mut b);
        edit(&app, &file, &mut b, "cell.set", json!({"cell": "B7", "input": "b's"}));
        take_applied(&mut a);
        take_applied(&mut b);
        app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Undo });
        take_applied(&mut a);
        take_applied(&mut b);
        let csv = |t: &Tab| {
            format!(
                "{:?}",
                t.session.active().unwrap().wb.sheets[0].cells.iter().map(|(c, x)| (c.a1(), format!("{:?}", x.value))).collect::<Vec<_>>()
            )
        };
        assert!(!csv(&b).contains("a again") && csv(&b).contains("b's"), "{}", csv(&b));
        assert_eq!(print(&a), print(&b));
        // b overwrites a's cell: a's undo is refused.
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "C8", "input": "a3"}));
        take_applied(&mut a);
        take_applied(&mut b);
        edit(&app, &file, &mut b, "cell.set", json!({"cell": "C8", "input": "b over"}));
        take_applied(&mut a);
        take_applied(&mut b);
        app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Undo });
        assert!(matches!(next(&mut a), ToClient::Message { text } if text == sync::UNDO_CHANGED));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn person(username: &str) -> Option<crate::auth::User> {
        Some(crate::auth::User { username: username.into(), name: username.into(), colour: "#000000".into(), owner: false })
    }

    /// Security review #3: a tab that reuses another tab's id (it is sent to everyone) gets its
    /// own connection name from the server, so it can't push that tab out, edit as it (when it
    /// may only view) or undo its changes.
    #[test]
    fn a_tab_cannot_pass_itself_off_as_another() {
        let (app, file, dir) = app();
        let mut ed = join_as(&app, &file, "c1", "tab1", person("ed"), true);
        let mut viewer = join_as(&app, &file, "c2", "tab1", person("vi"), false);
        // A second Join under a name already in the room is refused; the editor stays.
        let (out, rx) = unbounded_channel();
        app.rooms.send(&app, &file, RoomMsg::Join { client: "c1".into(), tab: "tab1".into(), user: person("vi"), can_edit: false, out, have: None });
        let mut late = Tab { name: "late".into(), rx, session: Session::new(), seq: 0, cseq: 0, seen: Vec::new() };
        assert!(matches!(next(&mut late), ToClient::Message { text } if text.contains("already open")));
        // The viewer's change is refused, whatever tab id it gave.
        edit(&app, &file, &mut viewer, "cell.set", json!({"cell": "A9", "input": "viewer"}));
        assert!(matches!(next(&mut viewer), ToClient::Rejected { .. }));
        // The editor still edits, and gets its own changes back as its own.
        edit(&app, &file, &mut ed, "cell.set", json!({"cell": "A5", "input": "ed's"}));
        assert_eq!(take_applied(&mut ed), ("c1".into(), 1));
        take_applied(&mut viewer);
        // Another editor with the same tab id can't undo the first editor's change.
        let mut mal = join_as(&app, &file, "c3", "tab1", person("mal"), true);
        app.rooms.send(&app, &file, RoomMsg::From { client: "c3".into(), msg: ToServer::Undo });
        assert!(matches!(next(&mut mal), ToClient::Message { text } if text == "Nothing to undo."));
        // Nor can a connection that isn't in the room.
        app.rooms.send(&app, &file, RoomMsg::From { client: "ghost".into(), msg: ToServer::Undo });
        // The editor's own undo works (the first thing after the refused ones).
        app.rooms.send(&app, &file, RoomMsg::From { client: "c1".into(), msg: ToServer::Undo });
        assert_eq!(take_applied(&mut ed).0, "", "undo comes from the server");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Security review #10: access is checked again while the file is open, at once after a
    /// sharing change (recheck_access) and every RECHECK anyway.
    #[test]
    fn access_is_checked_again_while_the_file_is_open() {
        use crate::sharing::{Item, Role, Share};
        let (app, file, dir) = app_with(true);
        crate::auth::add(&app.store, "boss", "Boss", "a long password").unwrap();
        let bob = crate::auth::add(&app.store, "bob", "Bob", "a long password").unwrap();
        let share = |role: Role| {
            let people = if role == Role::None { Vec::new() } else { vec![("bob".to_string(), role)] };
            crate::sharing::set(&app.store, Item::File(&file), &Share { people, link: Role::None }).unwrap();
        };
        share(Role::Edit);
        let mut b = join_as(&app, &file, "c1", "tab", Some(bob), true);
        // Made a viewer: told at once, with the file again and its own name.
        share(Role::View);
        app.rooms.recheck_access();
        match next(&mut b) {
            ToClient::Welcome { can_edit, you, .. } => {
                assert!(!can_edit);
                assert_eq!(you, "c1");
            }
            m => panic!("expected welcome, got {m:?}"),
        }
        edit(&app, &file, &mut b, "cell.set", json!({"cell": "A1", "input": "x"}));
        assert!(matches!(next(&mut b), ToClient::Rejected { .. }));
        // Share taken away, nobody says so: the periodic check finds it and disconnects.
        share(Role::None);
        assert!(matches!(next(&mut b), ToClient::NoAccess { .. }));
        for _ in 0..100 {
            match b.rx.try_recv() {
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(matches!(b.rx.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)), "the connection is ended");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A tab asking for the whole file over and over gets it at most once per RESYNC_GAP.
    #[test]
    fn resync_is_rate_limited() {
        let (app, file, dir) = app();
        let mut a = join(&app, &file, "a");
        for _ in 0..5 {
            app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Resync });
        }
        assert!(matches!(next(&mut a), ToClient::Reset { .. }));
        std::thread::sleep(Duration::from_millis(300));
        let mut resets = 0;
        while let Ok(m) = a.rx.try_recv() {
            resets += usize::from(matches!(serde_json::from_str(&m).unwrap(), ToClient::Reset { .. }));
        }
        assert_eq!(resets, 0, "not again straight away");
        std::thread::sleep(RESYNC_GAP);
        let mut resets = 0;
        while let Ok(m) = a.rx.try_recv() {
            resets += usize::from(matches!(serde_json::from_str(&m).unwrap(), ToClient::Reset { .. }));
        }
        assert_eq!(resets, 1, "the asks in between are answered once");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The file is saved open on the sheet of whoever changed it last, not the one it was first
    /// saved on (someone who added Sheet2 early saw an empty Sheet2 on every reopen).
    #[test]
    fn the_file_is_saved_open_where_the_last_change_was() {
        let dir = std::env::temp_dir().join(format!("starsheets-live-{}", crate::store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("home.insertSheet", json!({})).unwrap();
        s.execute("sheet.activate", json!({ "sheet": 1 })).unwrap();
        let bytes = gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "t.xlsx").unwrap();
        std::fs::write(dir.join("t.xlsx"), bytes).unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: false };
        let file = store.synced_index().unwrap().keys().next().unwrap().clone();
        let app =
            Arc::new(App { store, base: dir.clone(), lock: Mutex::new(()), location: String::new(), auth: false, site: None, rooms: Rooms::default() });
        let opened = |dir: &std::path::Path| {
            let bytes = std::fs::read(dir.join("t.xlsx")).unwrap();
            gridcraft_engine::io::open_bytes("t.xlsx", &bytes).unwrap().0.active_sheet()
        };
        assert_eq!(opened(&dir), 1);
        let mut a = join(&app, &file, "a");
        let sel = gridcraft_engine::selection::Selection::default();
        app.rooms.send(&app, &file, RoomMsg::From { client: "a".into(), msg: ToServer::Presence { sheet: 0, sel } });
        a.session.execute("sheet.activate", json!({ "sheet": 0 })).unwrap();
        a.session.sync_log = Some(Vec::new());
        edit(&app, &file, &mut a, "cell.set", json!({"cell": "A1", "input": "on Sheet1"}));
        take_applied(&mut a);
        app.rooms.send(&app, &file, RoomMsg::Leave { client: "a".into() });
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(opened(&dir), 0, "saved open on Sheet1, where the change was");
        let _ = std::fs::remove_dir_all(dir);
    }
}
