//! Everyone's own folder (sign-in on): `STORE/<username>/`, made when the account is, and the
//! one-time move of a server from before (everything at the top) into those folders. Also each
//! person's starred and recently opened items (`.starsheets/people/<username>.json`).
//!
//! The move, once, when the server's owner exists and `.starsheets/layout.json` doesn't say
//! "spaces" yet: a file at the top goes to the folder of whoever it belonged to (its old sharing
//! owner, if that account still exists), anything else to the server owner's folder. A file deeper
//! in a folder that someone else had made keeps that person as an editor, so nobody loses access.
//! Ids, versions and sharing stay as they are; only paths change.

use std::fs;
use std::io;

use serde_json::{Map, Value, json};

use crate::auth::User;
use crate::sharing::{self, Item, Role};
use crate::store::{Store, join, now};

fn layout_path(store: &Store) -> std::path::PathBuf {
    store.meta().join("layout.json")
}

pub fn migrated(store: &Store) -> bool {
    store.load_json(&layout_path(store)).get("layout").and_then(Value::as_str) == Some("spaces")
}

/// Makes `username`'s own folder if it isn't there.
pub fn ensure_space(store: &Store, username: &str) -> io::Result<()> {
    if store.spaces && !username.is_empty() {
        fs::create_dir_all(store.full(username))?;
    }
    Ok(())
}

/// Moves a server from before into people's own folders (see the top of this file). Returns how
/// many things were moved. Safe to call at every start: it does nothing once done.
pub fn migrate(store: &Store) -> io::Result<usize> {
    if !store.spaces || migrated(store) {
        return Ok(0);
    }
    let users = crate::auth::users(store);
    let Some(owner) = users.iter().find(|u| u.owner).map(|u| u.username.clone()) else { return Ok(0) };
    let index = store.synced_index()?;
    store.synced_folders()?;
    let names: Vec<String> = users.iter().map(|u| u.username.clone()).collect();
    // Old per-file owners, before paths change.
    let old_owner: Vec<(String, String, String)> = index
        .iter()
        .filter_map(|(id, path)| sharing::old_owner(store, id).filter(|o| names.contains(o)).map(|o| (id.clone(), path.clone(), o)))
        .collect();
    // A top-level folder already named after a person would be taken for their own folder:
    // those are moved too (into the owner's folder), so first get them out of the way.
    let mut tops: Vec<String> =
        fs::read_dir(&store.root)?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect();
    tops.sort();
    let staging = format!(".moving-{}", crate::store::random_hex(4));
    fs::create_dir_all(store.full(&staging))?;
    for name in &tops {
        fs::rename(store.full(name), store.full(&staging).join(name))?;
    }
    for u in &names {
        ensure_space(store, u)?;
    }
    let mut moves: Vec<(String, String)> = Vec::new();
    for name in &tops {
        let from = format!("{staging}/{name}");
        // A file at the top goes home to whoever it belonged to.
        let who = old_owner.iter().find(|(_, p, _)| p == name).map_or(owner.clone(), |(_, _, o)| o.clone());
        let is_dir = store.full(&from).is_dir();
        let target = free_name(store, &who, name, is_dir)?;
        fs::rename(store.full(&from), store.full(&join(&who, &target)))?;
        moves.push((name.clone(), join(&who, &target)));
    }
    // All at once: a later move must not catch a path an earlier one produced.
    rewrite_paths(store, &moves)?;
    let moved = moves.len();
    let _ = fs::remove_dir(store.full(&staging));
    // Bin: the old shared bin goes into the owner's bin.
    let old_bin = store.root.join(".bin");
    if old_bin.is_dir() {
        let new_bin = store.full(&owner).join(".bin");
        fs::create_dir_all(&new_bin)?;
        let mut bins = store.load_json(&store.meta().join("bin.json"));
        for e in fs::read_dir(&old_bin)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            fs::rename(e.path(), new_bin.join(&name))?;
            if let Some(mut info) = bins.remove(&name) {
                if let Some(o) = info.get("original").and_then(Value::as_str) {
                    info["original"] = json!(join(&owner, o));
                }
                bins.insert(format!("{owner}/{name}"), info);
            }
        }
        store.save_json(&store.meta().join("bin.json"), &Value::Object(bins))?;
        let _ = fs::remove_dir(&old_bin);
    }
    // Files someone else made inside the owner's folders: they keep editing them.
    let index = store.synced_index()?;
    for (id, _, o) in &old_owner {
        if let Some(path) = index.get(id)
            && crate::store::space_of(path) != o
        {
            let mut share = sharing::get(store, Item::File(id));
            if !share.people.iter().any(|(p, _)| p == o) {
                share.people.push((o.clone(), Role::Edit));
                sharing::set(store, Item::File(id), &share)?;
            }
        }
    }
    store.save_json(&layout_path(store), &json!({ "layout": "spaces", "since": now() as i64, "moved": moved }))?;
    Ok(moved)
}

/// After `username`'s account is removed: their folder goes into the server owner's folder as
/// "Removed – <username>" (nothing is lost, and the owner decides what to keep), they are taken
/// off every share, and their stars and recent list go. A new account with the same name then
/// starts empty. Returns where the folder went.
pub fn retire(store: &Store, username: &str) -> io::Result<Option<String>> {
    sharing::forget_person(store, username)?;
    let _ = fs::remove_file(prefs_path(store, username));
    let owner = sharing::server_owner(store);
    if !store.spaces || username.is_empty() || owner.is_empty() || owner == username || !store.full(username).is_dir() {
        return Ok(None);
    }
    // Ids first, so the files and folders keep theirs (versions, and shares they gave, stay).
    store.synced_index()?;
    store.synced_folders()?;
    let name = free_name(store, &owner, &format!("Removed – {username}"), true)?;
    let to = join(&owner, &name);
    fs::rename(store.full(username), store.full(&to))?;
    rewrite_paths(store, &[(username.to_string(), to.clone())])?;
    // Their bin went along inside the folder; the bin list forgets it.
    let bin_index = store.meta().join("bin.json");
    let mut bins = store.load_json(&bin_index);
    let before = bins.len();
    bins.retain(|k, _| !k.starts_with(&format!("{username}/")));
    if bins.len() != before {
        store.save_json(&bin_index, &Value::Object(bins))?;
    }
    Ok(Some(to))
}

fn free_name(store: &Store, space: &str, name: &str, is_dir: bool) -> io::Result<String> {
    let (stem, ext) = if is_dir { (name.to_string(), String::new()) } else { split_ext(name) };
    for n in 1..10000 {
        let candidate = if n == 1 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        if !store.full(space).join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::other("no free name"))
}

fn split_ext(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// For every (top name, new path): points the ids at that name (and below) to the new path (and
/// below), in the file and folder indexes, in one pass.
fn rewrite_paths(store: &Store, moves: &[(String, String)]) -> io::Result<()> {
    for file in ["index.json", "folders.json"] {
        let path = store.meta().join(file);
        let mut ids = store.load_json(&path);
        for v in ids.values_mut() {
            let Some(p) = v.as_str() else { continue };
            let (top, tail) = p.split_once('/').map_or((p, None), |(t, r)| (t, Some(r)));
            if let Some((_, new)) = moves.iter().find(|(old, _)| old == top) {
                *v = json!(tail.map_or(new.clone(), |t| format!("{new}/{t}")));
            }
        }
        store.save_json(&path, &Value::Object(ids))?;
    }
    Ok(())
}

// ---------------------------------------------------------------- who changed what last

fn changed_path(store: &Store) -> std::path::PathBuf {
    store.meta().join("changed.json")
}

/// Notes that `username` saved a change to `file` just now (for Details: "Modified … by …").
pub fn note_changed(store: &Store, file: &str, username: &str) -> io::Result<()> {
    let mut all = store.load_json(&changed_path(store));
    all.insert(file.to_string(), json!({ "by": username, "at": now() as i64 }));
    store.save_json(&changed_path(store), &Value::Object(all))
}

/// Who last changed `file` (username) and when (Unix seconds), where known.
pub fn last_change(store: &Store, file: &str) -> Option<(String, i64)> {
    let all = store.load_json(&changed_path(store));
    let v = all.get(file)?;
    Some((v.get("by")?.as_str()?.to_string(), v.get("at").and_then(Value::as_i64).unwrap_or(0)))
}

/// Who last changed each file (file id → username), where known.
pub fn changed_by(store: &Store) -> Map<String, Value> {
    store.load_json(&changed_path(store)).into_iter().filter_map(|(k, v)| Some((k, v.get("by")?.clone()))).collect()
}

// ---------------------------------------------------------------- starred and recent

fn prefs_path(store: &Store, username: &str) -> std::path::PathBuf {
    store.meta().join("people").join(format!("{username}.json"))
}

fn prefs(store: &Store, username: &str) -> Map<String, Value> {
    store.load_json(&prefs_path(store, username))
}

fn save_prefs(store: &Store, username: &str, p: &Map<String, Value>) -> io::Result<()> {
    fs::create_dir_all(store.meta().join("people"))?;
    store.save_json(&prefs_path(store, username), &Value::Object(p.clone()))
}

/// Starred items as "f:<file id>" / "d:<folder id>".
pub fn starred(store: &Store, user: &User) -> Vec<String> {
    prefs(store, &user.username)
        .get("starred")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn set_star(store: &Store, user: &User, item: &str, on: bool) -> io::Result<()> {
    let mut p = prefs(store, &user.username);
    let mut list = starred(store, user);
    list.retain(|i| i != item);
    if on {
        list.insert(0, item.to_string());
    }
    p.insert("starred".into(), json!(list));
    save_prefs(store, &user.username, &p)
}

/// Folder colours `user` picked (folder id → "#RRGGBB"). Personal, as in Drive: they only change
/// how the folders look to that person.
pub fn colours(store: &Store, user: &User) -> Map<String, Value> {
    prefs(store, &user.username).get("colours").and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Sets (or with "" clears) a folder's colour for `user`. Only "#RRGGBB" is kept.
pub fn set_colour(store: &Store, user: &User, folder: &str, colour: &str) -> io::Result<()> {
    let mut p = prefs(store, &user.username);
    let mut all = colours(store, user);
    let ok = colour.len() == 7 && colour.starts_with('#') && colour[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        all.insert(folder.to_string(), json!(colour.to_uppercase()));
    } else {
        all.remove(folder);
    }
    p.insert("colours".into(), Value::Object(all));
    save_prefs(store, &user.username, &p)
}

/// When `user` last opened each file (file id → Unix seconds), newest kept to 100.
pub fn opened(store: &Store, user: &User) -> Map<String, Value> {
    prefs(store, &user.username).get("opened").and_then(Value::as_object).cloned().unwrap_or_default()
}

pub fn note_opened(store: &Store, user: &User, file: &str) -> io::Result<()> {
    let mut p = prefs(store, &user.username);
    let mut list: Vec<(String, i64)> =
        opened(store, user).into_iter().filter_map(|(k, v)| Some((k, v.as_i64()?))).filter(|(k, _)| k != file).collect();
    list.push((file.to_string(), now() as i64));
    list.sort_by_key(|(_, t)| -t);
    list.truncate(100);
    p.insert("opened".into(), Value::Object(list.into_iter().map(|(k, t)| (k, json!(t))).collect()));
    save_prefs(store, &user.username, &p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth;

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("starsheets-spaces-{}", crate::store::random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        Store { root: dir, keep: 2, spaces: true }
    }

    #[test]
    fn a_server_from_before_moves_into_peoples_folders() {
        let s = store();
        let r = &s.root;
        fs::create_dir_all(r.join("Work/Deep")).unwrap();
        fs::create_dir_all(r.join("ann")).unwrap(); // a folder that happens to share a name
        fs::write(r.join("Budget.xlsx"), "b").unwrap();
        fs::write(r.join("Anns.xlsx"), "a").unwrap();
        fs::write(r.join("Work/Deep/Plan.xlsx"), "p").unwrap();
        fs::write(r.join("ann/Thing.xlsx"), "t").unwrap();
        fs::create_dir_all(r.join(".bin")).unwrap();
        fs::write(r.join(".bin/Old.xlsx"), "o").unwrap();
        s.save_json(&s.meta().join("bin.json"), &json!({"Old.xlsx": {"original": "Old.xlsx", "deleted": now() as i64}})).unwrap();
        let index = s.synced_index().unwrap();
        let id = |p: &str| index.iter().find(|(_, v)| *v == p).unwrap().0.clone();
        let (budget, anns, plan) = (id("Budget.xlsx"), id("Anns.xlsx"), id("Work/Deep/Plan.xlsx"));
        auth::add(&s, "boss", "Boss", "bosspassword").unwrap();
        auth::add(&s, "ann", "Ann", "annpassword").unwrap();
        // Before: per-file owners (Ann made two of them).
        s.save_json(
            &s.meta().join("sharing.json"),
            &json!({"files": {
                anns.clone(): {"owner": "ann", "people": [], "link": "none"},
                plan.clone(): {"owner": "ann", "people": [], "link": "none"},
            }}),
        )
        .unwrap();
        assert_eq!(migrate(&s).unwrap(), 4, "Budget, Anns, Work, ann");
        assert_eq!(migrate(&s).unwrap(), 0, "only once");
        let index = s.synced_index().unwrap();
        assert_eq!(index[&budget], "boss/Budget.xlsx");
        assert_eq!(index[&anns], "ann/Anns.xlsx", "a top file goes home to whoever made it");
        assert_eq!(index[&plan], "boss/Work/Deep/Plan.xlsx");
        assert!(r.join("boss/ann/Thing.xlsx").is_file(), "a folder named like a person isn't mistaken for theirs");
        assert!(r.join("boss/.bin/Old.xlsx").is_file());
        assert!(s.load_json(&s.meta().join("bin.json")).contains_key("boss/Old.xlsx"));
        // Ann still edits the file she made inside Boss's folder.
        let ann = auth::users(&s).into_iter().find(|u| u.username == "ann").unwrap();
        assert_eq!(sharing::file_role(&s, true, Some(&ann), &plan), Role::Edit);
        assert_eq!(sharing::file_role(&s, true, Some(&ann), &anns), Role::Owner);
        assert_eq!(sharing::file_role(&s, true, Some(&ann), &budget), Role::None);
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn a_removed_account_leaves_nothing_to_a_new_one_of_the_same_name() {
        let s = store();
        auth::add(&s, "boss", "Boss", "bosspassword").unwrap();
        auth::add(&s, "ann", "Ann", "annpassword").unwrap();
        let bob = auth::add(&s, "bob", "Bob", "bobpassword").unwrap();
        migrate(&s).unwrap();
        fs::create_dir_all(s.root.join("ann/Team")).unwrap();
        fs::write(s.root.join("ann/Team/Plan.xlsx"), "p").unwrap();
        fs::create_dir_all(s.root.join("boss/Shared")).unwrap();
        fs::write(s.root.join("boss/Shared/Q1.xlsx"), "q").unwrap();
        fs::create_dir_all(s.root.join("boss/Removed – bob")).unwrap(); // the name is taken
        let index = s.synced_index().unwrap();
        let id = |p: &str| index.iter().find(|(_, v)| *v == p).unwrap().0.clone();
        let (plan, q1) = (id("ann/Team/Plan.xlsx"), id("boss/Shared/Q1.xlsx"));
        let folders = s.synced_folders().unwrap();
        let shared = folders.iter().find(|(_, p)| *p == "boss/Shared").unwrap().0.clone();
        sharing::update(&s, Item::File(&q1), "boss/Shared/Q1.xlsx", &json!({"people": {"bob": "edit", "ann": "view"}})).unwrap();
        sharing::update(&s, Item::Folder(&shared), "boss/Shared", &json!({"people": {"bob": "view"}})).unwrap();
        set_star(&s, &bob, "f:x", true).unwrap();

        auth::remove(&s, "bob").unwrap();
        assert_eq!(retire(&s, "bob").unwrap().as_deref(), Some("boss/Removed – bob (2)"), "a free name");
        assert!(sharing::get(&s, Item::File(&q1)).people == vec![("ann".to_string(), Role::View)], "taken off shares, others kept");
        assert!(sharing::get(&s, Item::Folder(&shared)).people.is_empty());
        let bob2 = auth::add(&s, "bob", "New Bob", "bobpassword2").unwrap();
        assert_eq!(sharing::file_role(&s, true, Some(&bob2), &q1), Role::None, "a new bob gets nothing of the old one's");
        assert!(starred(&s, &bob2).is_empty());

        auth::remove(&s, "ann").unwrap();
        assert_eq!(retire(&s, "ann").unwrap().as_deref(), Some("boss/Removed – ann"));
        assert!(!s.root.join("ann").exists() && s.root.join("boss/Removed – ann/Team/Plan.xlsx").is_file());
        assert_eq!(s.synced_index().unwrap()[&plan], "boss/Removed – ann/Team/Plan.xlsx", "the id goes with it");
        let ann2 = auth::add(&s, "ann", "New Ann", "annpassword2").unwrap();
        ensure_space(&s, "ann").unwrap();
        assert_eq!(sharing::file_role(&s, true, Some(&ann2), &plan), Role::None);
        // A second removal of the same name gets its own folder.
        fs::write(s.root.join("ann/New.xlsx"), "n").unwrap();
        auth::remove(&s, "ann").unwrap();
        assert_eq!(retire(&s, "ann").unwrap().as_deref(), Some("boss/Removed – ann (2)"));
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn folder_sharing_covers_whats_inside() {
        let s = store();
        let boss = auth::add(&s, "boss", "Boss", "bosspassword").unwrap();
        let ann = auth::add(&s, "ann", "Ann", "annpassword").unwrap();
        let bob = auth::add(&s, "bob", "Bob", "bobpassword").unwrap();
        migrate(&s).unwrap();
        fs::create_dir_all(s.root.join("ann/Team/Sub")).unwrap();
        fs::write(s.root.join("ann/Team/Sub/Q1.xlsx"), "q").unwrap();
        fs::write(s.root.join("ann/Private.xlsx"), "p").unwrap();
        let index = s.synced_index().unwrap();
        let id = |p: &str| index.iter().find(|(_, v)| *v == p).unwrap().0.clone();
        let (q1, private) = (id("ann/Team/Sub/Q1.xlsx"), id("ann/Private.xlsx"));
        let team = s.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Team").unwrap().0;
        assert_eq!(sharing::file_role(&s, true, Some(&bob), &q1), Role::None);
        sharing::update(&s, Item::Folder(&team), "ann/Team", &json!({"people": {"bob": "edit"}})).unwrap();
        let a = sharing::Access::new(&s, true, Some(&bob));
        assert_eq!(a.file_role(&q1), Role::Edit, "inside a shared folder");
        assert_eq!(a.folder_role("ann/Team/Sub"), Role::Edit);
        assert_eq!(a.file_role(&private), Role::None);
        assert!(a.file_listed(&q1) && !a.file_listed(&private) && !a.folder_listed("ann"));
        assert_eq!(a.shared_roots(), (vec!["ann/Team".to_string()], vec![]));
        assert_eq!(sharing::Access::new(&s, true, Some(&boss)).file_role(&private), Role::Owner, "the server's owner");
        assert_eq!(sharing::Access::new(&s, true, Some(&ann)).folder_role("ann"), Role::Owner);
        // Starred and recent are per person.
        set_star(&s, &bob, "d:x", true).unwrap();
        note_opened(&s, &bob, &q1).unwrap();
        assert_eq!(starred(&s, &bob), vec!["d:x".to_string()]);
        assert!(starred(&s, &ann).is_empty() && opened(&s, &bob).contains_key(&q1));
        let _ = fs::remove_dir_all(&s.root);
    }
}
