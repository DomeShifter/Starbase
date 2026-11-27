//! Who may open which file or folder, as in Google Drive. Everyone has their own folder at the
//! top (`<username>/`, see store.rs); what's in it is theirs. A file or a folder can be shared
//! with people (can view, can comment or can edit) and have a general setting ("anyone with the link", meaning
//! anyone signed in to this server who has the link). Sharing a folder shares everything in it,
//! also what's added later. Kept in STORE/.starsheets/sharing.json by file id ("files") and
//! folder id ("folders").
//!
//! The server's owner (the first account, who runs the server) can open and manage everything.
//! With sign-in off everyone may do everything, as before.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::auth::{self, User};
use crate::store::Store;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    #[default]
    None,
    View,
    /// Can view, and add, reply to and resolve comments in a document, not change its text
    /// (commenting.rs checks their saves). Spreadsheets treat a commenter as a viewer.
    Comment,
    Edit,
    Owner,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::None => "none",
            Role::View => "view",
            Role::Comment => "comment",
            Role::Edit => "edit",
            Role::Owner => "owner",
        }
    }
    fn parse(s: &str) -> Role {
        match s {
            "view" => Role::View,
            "comment" => Role::Comment,
            "edit" => Role::Edit,
            _ => Role::None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Share {
    /// username → View or Edit, in the order they were added.
    pub people: Vec<(String, Role)>,
    /// What anyone signed in with the link may do (None = restricted).
    pub link: Role,
    /// Keep where a photo was taken when someone else downloads it (off by default).
    pub keep_location: bool,
}

impl Share {
    pub fn is_shared(&self) -> bool {
        !self.people.is_empty() || self.link != Role::None
    }
}

/// A file or a folder, by id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item<'a> {
    File(&'a str),
    Folder(&'a str),
}

impl Item<'_> {
    fn group(self) -> &'static str {
        match self {
            Item::File(_) => "files",
            Item::Folder(_) => "folders",
        }
    }
    fn id(self) -> String {
        match self {
            Item::File(id) | Item::Folder(id) => id.to_string(),
        }
    }
}

fn path(store: &Store) -> std::path::PathBuf {
    store.meta().join("sharing.json")
}

fn load(store: &Store) -> Map<String, Value> {
    store.load_json(&path(store))
}

fn parse(v: &Value) -> Share {
    let people = v.get("people").and_then(Value::as_array).cloned().unwrap_or_default();
    Share {
        people: people
            .iter()
            .filter_map(|p| Some((p.get("username")?.as_str()?.to_string(), Role::parse(p.get("role")?.as_str()?))))
            .filter(|(_, r)| *r != Role::None)
            .collect(),
        link: Role::parse(v.get("link").and_then(Value::as_str).unwrap_or("none")),
        keep_location: v.get("keep_location").and_then(Value::as_bool).unwrap_or(false),
    }
}

pub fn get(store: &Store, item: Item) -> Share {
    load(store).get(item.group()).and_then(|g| g.get(item.id())).map(parse).unwrap_or_default()
}

pub fn set(store: &Store, item: Item, share: &Share) -> std::io::Result<()> {
    let mut all = load(store);
    let mut group = all.get(item.group()).and_then(Value::as_object).cloned().unwrap_or_default();
    if share.is_shared() {
        let people: Vec<Value> = share.people.iter().map(|(u, r)| json!({ "username": u, "role": r.name() })).collect();
        group.insert(item.id(), json!({ "people": people, "link": share.link.name(), "keep_location": share.keep_location }));
    } else {
        group.remove(&item.id());
    }
    all.insert(item.group().to_string(), Value::Object(group));
    store.save_json(&path(store), &Value::Object(all))
}

/// Takes `username` off every file and folder share (their account was removed), so a new
/// account with the same name gets nothing of theirs.
pub fn forget_person(store: &Store, username: &str) -> std::io::Result<()> {
    let mut all = load(store);
    let mut changed = false;
    for group in ["files", "folders"] {
        let Some(Value::Object(items)) = all.get_mut(group) else { continue };
        for item in items.values_mut() {
            if let Some(Value::Array(people)) = item.get_mut("people") {
                let before = people.len();
                people.retain(|p| p.get("username").and_then(Value::as_str) != Some(username));
                changed |= people.len() != before;
            }
        }
    }
    if changed { store.save_json(&path(store), &Value::Object(all)) } else { Ok(()) }
}

/// The old per-file owner (from before everyone had a folder), for moving files home once.
pub fn old_owner(store: &Store, file: &str) -> Option<String> {
    load(store).get("files")?.get(file)?.get("owner")?.as_str().map(str::to_string)
}

pub fn server_owner(store: &Store) -> String {
    auth::users(store).into_iter().find(|u| u.owner).map(|u| u.username).unwrap_or_default()
}

/// Who owns a path: the person whose folder it's in (or the server's owner without spaces).
pub fn owner_of(store: &Store, relpath: &str) -> String {
    if store.spaces { crate::store::space_of(relpath).to_string() } else { server_owner(store) }
}

/// What one person may do, worked out once per request: their own folder, the server owner's
/// rights, and every share that names them or has a link.
pub struct Access {
    auth: bool,
    pub user: Option<User>,
    files: HashMap<String, Share>,
    /// Folder path → its share.
    folders: HashMap<String, Share>,
    /// Folder path → id, and file id → path.
    pub folder_ids: HashMap<String, String>,
    pub file_paths: HashMap<String, String>,
    hidden: std::collections::HashSet<String>,
    private_roots: Vec<String>,
    photo_root: std::sync::OnceLock<Option<crate::mounts::Root>>,
}

impl Access {
    pub fn new(store: &Store, auth: bool, user: Option<&User>) -> Access {
        Self::build(store, auth, user, false)
    }

    /// Like `new`, but `user`'s own locked pictures show too while their Locked is open.
    pub fn with_locked(store: &Store, auth: bool, user: Option<&User>) -> Access {
        Self::build(store, auth, user, true)
    }

    fn build(store: &Store, auth: bool, user: Option<&User>, with_mine: bool) -> Access {
        let all = load(store);
        let group = |g: &str| -> HashMap<String, Share> {
            all.get(g).and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), parse(v))).collect()).unwrap_or_default()
        };
        let private_roots: Vec<_> =
            user.filter(|_| auth && store.spaces).map(|user| crate::mounts::choices(store, user).into_values().collect()).unwrap_or_default();
        let allowed = |path: &str| !crate::mounts::private_path(path) || private_roots.iter().any(|root| crate::mounts::within(path, root));
        let folder_ids: HashMap<String, String> =
            store.synced_folders().unwrap_or_default().into_iter().filter(|(_, p)| allowed(p)).map(|(id, p)| (p, id)).collect();
        let by_id = group("folders");
        let folders = folder_ids.iter().filter_map(|(p, id)| Some((p.clone(), by_id.get(id)?.clone()))).collect();
        let mut hidden = crate::photos::locked::hidden_for(store, user.map(|u| u.username.as_str()), with_mine);
        let file_paths = store
            .synced_index()
            .unwrap_or_default()
            .into_iter()
            .filter(|(id, p)| {
                if !allowed(p) {
                    hidden.insert(id.clone());
                }
                !hidden.contains(id)
            })
            .collect();
        Access {
            auth,
            user: user.cloned(),
            files: group("files"),
            folders,
            folder_ids,
            file_paths,
            hidden,
            private_roots,
            photo_root: std::sync::OnceLock::new(),
        }
    }

    pub fn photo_root(&self, store: &Store) -> Option<&crate::mounts::Root> {
        self.photo_root.get_or_init(|| self.user.as_ref().and_then(|u| crate::mounts::configured_photos(store, u))).as_ref()
    }

    pub fn photo_allowed(&self, store: &Store, path: &str) -> bool {
        match self.photo_root(store) {
            None => !crate::mounts::private_path(path),
            Some(crate::mounts::Root::Available(root)) => crate::mounts::within(path, root),
            Some(crate::mounts::Root::Unavailable) => false,
        }
    }

    fn me(&self) -> &str {
        self.user.as_ref().map_or("", |u| u.username.as_str())
    }

    /// Whether this is someone who can do everything (sign-in off, or the server's owner).
    pub fn admin(&self) -> bool {
        !self.auth || self.user.as_ref().is_some_and(|u| u.owner)
    }

    /// The shares that apply at `relpath` (a folder or a file's folder) and above.
    fn shares_above<'a>(&'a self, relpath: &str) -> impl Iterator<Item = &'a Share> + 'a {
        let mut paths = Vec::new();
        let mut p = relpath.to_string();
        while !p.is_empty() {
            paths.push(p.clone());
            p = crate::store::parent_of(&p);
        }
        paths.into_iter().filter_map(move |p| self.folders.get(&p))
    }

    fn role_from(&self, shares: impl Iterator<Item = Share>, named_only: bool) -> Role {
        let me = self.me().to_string();
        shares
            .map(|s| {
                let mine = s.people.iter().find(|(p, _)| *p == me).map_or(Role::None, |p| p.1);
                if named_only { mine } else { mine.max(s.link) }
            })
            .max()
            .unwrap_or(Role::None)
    }

    /// What I may do with the folder at `relpath` ("" = the top).
    pub fn folder_role(&self, relpath: &str) -> Role {
        if crate::mounts::private_path(relpath) {
            return if self.private_roots.iter().any(|root| crate::mounts::within(relpath, root)) { Role::Owner } else { Role::None };
        }
        if self.admin() {
            return Role::Owner;
        }
        if self.user.is_none() || relpath.is_empty() {
            return Role::None;
        }
        if crate::store::space_of(relpath) == self.me() {
            return Role::Owner;
        }
        self.role_from(self.shares_above(relpath).cloned(), false)
    }

    /// What I may do with a file.
    pub fn file_role(&self, file: &str) -> Role {
        if self.hidden.contains(file) {
            return Role::None;
        }
        if self.admin() {
            return Role::Owner;
        }
        let Some(path) = self.file_paths.get(file) else { return Role::None };
        if self.user.is_none() {
            return Role::None;
        }
        if crate::store::space_of(path) == self.me() {
            return Role::Owner;
        }
        let own = self.files.get(file).cloned().into_iter();
        self.role_from(own.chain(self.shares_above(&crate::store::parent_of(path)).cloned()), false)
    }

    /// Whether a photo's location may go with it to people who aren't its owner: the file's own
    /// share, or a folder it's in, says to keep it.
    pub fn keeps_location(&self, file: &str) -> bool {
        let Some(path) = self.file_paths.get(file) else { return false };
        self.files.get(file).is_some_and(|s| s.keep_location) || self.shares_above(&crate::store::parent_of(path)).any(|s| s.keep_location)
    }

    /// Whether a file shows in my lists: mine, or shared with me by name (itself or a folder it's
    /// in). A file opened only through its link isn't listed, as in Drive.
    pub fn file_listed(&self, file: &str) -> bool {
        let Some(path) = self.file_paths.get(file) else { return false };
        if self.admin() || crate::store::space_of(path) == self.me() {
            return true;
        }
        let own = self.files.get(file).cloned().into_iter();
        self.role_from(own.chain(self.shares_above(&crate::store::parent_of(path)).cloned()), true) > Role::None
    }

    pub fn folder_listed(&self, relpath: &str) -> bool {
        if crate::mounts::private_path(relpath) {
            return self.folder_role(relpath) == Role::Owner;
        }
        if self.admin() || crate::store::space_of(relpath) == self.me() {
            return true;
        }
        self.role_from(self.shares_above(relpath).cloned(), true) > Role::None
    }

    /// Whether this item has its own share (shown as a "shared" mark).
    pub fn file_shared(&self, file: &str) -> bool {
        self.files.get(file).is_some_and(Share::is_shared)
    }
    pub fn folder_shared(&self, relpath: &str) -> bool {
        self.folders.get(relpath).is_some_and(Share::is_shared)
    }

    /// Shared with me by name and not inside something else shared with me: the tops of "Shared
    /// with me". Folders as paths, files as ids.
    pub fn shared_roots(&self) -> (Vec<String>, Vec<String>) {
        let me = self.me().to_string();
        let named = |s: &Share| s.people.iter().any(|(p, _)| *p == me);
        let not_mine = |p: &str| crate::store::space_of(p) != me;
        let folder_named = |p: &str| self.folders.get(p).is_some_and(named);
        let above_named = |p: &str| {
            let mut q = crate::store::parent_of(p);
            while !q.is_empty() {
                if folder_named(&q) {
                    return true;
                }
                q = crate::store::parent_of(&q);
            }
            false
        };
        let mut folders: Vec<String> =
            self.folders.iter().filter(|(p, s)| named(s) && not_mine(p) && !above_named(p)).map(|(p, _)| p.clone()).collect();
        let mut files: Vec<String> = self
            .files
            .iter()
            .filter(|(id, s)| {
                named(s) && self.file_paths.get(*id).is_some_and(|p| not_mine(p) && !folder_named(&crate::store::parent_of(p)) && !above_named(p))
            })
            .map(|(id, _)| id.clone())
            .collect();
        folders.sort();
        files.sort();
        (folders, files)
    }
}

/// What `user` may do with `file` (one-off check, e.g. joining a live file).
pub fn file_role(store: &Store, auth: bool, user: Option<&User>, file: &str) -> Role {
    Access::new(store, auth, user).file_role(file)
}

/// The sharing settings as the Share dialog shows them; `relpath` says whose it is.
pub fn describe(store: &Store, item: Item, relpath: &str, you: Role) -> Value {
    let share = get(store, item);
    let users = auth::users(store);
    let person = |username: &str, role: &str| {
        let u = users.iter().find(|u| u.username == username);
        json!({ "username": username, "name": u.map_or(username, |u| u.name.as_str()), "colour": u.map_or("#5F6368", |u| u.colour.as_str()), "role": role })
    };
    let owner = owner_of(store, relpath);
    let mut people = vec![person(&owner, "owner")];
    people.extend(share.people.iter().filter(|(u, _)| *u != owner).map(|(u, r)| person(u, r.name())));
    let public = match item {
        Item::File(id) => crate::publiclink::describe(store, id),
        Item::Folder(_) => json!({ "on": false }),
    };
    json!({ "people": people, "link": share.link.name(), "keep_location": share.keep_location, "you": you.name(), "folder": matches!(item, Item::Folder(_)), "public": public })
}

/// Applies the Share dialog's changes: `people` maps username → "view", "comment", "edit" or
/// "remove"; `link` is "none", "view", "comment" or "edit".
pub fn update(store: &Store, item: Item, relpath: &str, body: &Value) -> Result<(), String> {
    if crate::mounts::private_path(relpath) {
        return Err("Files and folders on account mounts are private.".into());
    }
    let mut share = get(store, item);
    let owner = owner_of(store, relpath);
    let known: Vec<String> = auth::users(store).into_iter().map(|u| u.username).collect();
    if let Some(people) = body.get("people").and_then(Value::as_object) {
        for (username, role) in people {
            let role = role.as_str().unwrap_or("");
            if *username == owner {
                continue;
            }
            share.people.retain(|(u, _)| u != username);
            if role != "remove" {
                if !known.contains(username) {
                    return Err(format!("There's no account called {username}."));
                }
                let r = Role::parse(role);
                if r == Role::None {
                    return Err("A person can view, comment or edit.".into());
                }
                share.people.push((username.clone(), r));
            }
        }
    }
    if let Some(link) = body.get("link").and_then(Value::as_str) {
        share.link = Role::parse(link);
    }
    if let Some(k) = body.get("keep_location").and_then(Value::as_bool) {
        share.keep_location = k;
    }
    if let Item::File(id) = item {
        crate::publiclink::update(store, id, body)?;
    }
    set(store, item, &share).map_err(|e| e.to_string())
}

// Tests: spaces.rs (folder sharing, migration) and parity.py (the HTTP side).
