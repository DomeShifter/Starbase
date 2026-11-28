//! Resolve My Files and Photos settings against administrator-provisioned bind mounts.
//!
//! The deployment administrator mounts a directory under `Store::root`, inside the
//! target user's space in a dedicated subdirectory without nested mounts, and records it in `.starsheets/mounts.json`. For example:
//! `{"<account key>": {"library": "ann/starbase-mounts/Photo Library"}}`. The account key is stored in
//! the user's preferences and changes if the account is removed and recreated.
//!
//! The client selects an assigned ID or a folder inside My Files. These functions
//! resolve settings. File, folder and background-worker access must enforce the
//! resulting boundaries before a route exposes the settings.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::{self, User};
use crate::spaces;
use crate::store::{self, Store};

/// This namespace stays private even if an assignment is removed or its mount disappears.
pub(crate) const DIRECTORY: &str = "starbase-mounts";

pub fn private_path(path: &str) -> bool {
    path.split('/').nth(1) == Some(DIRECTORY)
}

pub fn account_mount_dir(username: &str) -> String {
    store::join(username, DIRECTORY)
}

pub fn within(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|tail| tail.starts_with('/'))
}

const KEY: &str = "mount_account";
const SELECTION: &str = "library_roots";

/// A Photos source can be an assigned bind mount or an existing folder relative to My Files.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhotosChoice {
    Mount(String),
    Folder(String),
}

/// A missing or revoked selected root stays unavailable until the account changes its selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Root {
    Available(String),
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    pub my_files: Root,
    pub photos: Root,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct Assignment {
    id: String,
    path: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SavedPhotos {
    Mount(Assignment),
    Folder(String),
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedRoots {
    my_files: Option<Assignment>,
    photos: Option<SavedPhotos>,
}

#[derive(Debug)]
struct Mount {
    device: Vec<u8>,
    source: PathBuf,
}

/// The same container path may have stacked mounts. Refuse ambiguous locations.
type MountTable = HashMap<PathBuf, Option<Mount>>;

/// Generate and persist a provisioning key for the test fixture.
#[cfg(test)]
pub fn account_key(store: &Store, user: &User) -> io::Result<String> {
    check_account(store, user)?;
    let mut prefs = spaces::prefs(store, &user.username);
    if let Some(key) = prefs.get(KEY).and_then(Value::as_str).filter(|key| valid_key(key)) {
        return Ok(key.to_string());
    }
    let key = store::random_hex(16);
    prefs.insert(KEY.to_string(), json!(key));
    spaces::save_prefs(store, &user.username, &prefs)?;
    Ok(key)
}

/// The verified IDs and container-relative targets assigned to this account.
pub fn choices(store: &Store, user: &User) -> BTreeMap<String, String> {
    if !valid_account_name(store, user) {
        return BTreeMap::new();
    }
    let prefs = spaces::prefs(store, &user.username);
    prefs.get(KEY).map_or_else(BTreeMap::new, |key| choices_with(store, user, Some(key), &mount_table()))
}

/// Report the caller's choices and effective paths, including an unavailable saved selection.
pub fn settings(store: &Store, user: &User) -> Value {
    let prefs = spaces::prefs(store, &user.username);
    let selected = prefs.get(SELECTION).and_then(|v| serde_json::from_value::<SavedRoots>(v.clone()).ok()).unwrap_or_default();
    let photos = selected.photos.map(|choice| match choice {
        SavedPhotos::Folder(path) => PhotosChoice::Folder(path),
        SavedPhotos::Mount(assignment) => PhotosChoice::Mount(assignment.id),
    });
    let path = |root| match root {
        Root::Available(path) => Some(path),
        Root::Unavailable => None,
    };
    let current = roots(store, user);
    json!({
        "mounts": choices(store, user),
        "selected": {"my_files": selected.my_files.map(|a| a.id), "photos": photos},
        "roots": {"my_files": path(current.my_files), "photos": path(current.photos)},
    })
}

/// Read current roots. With no preference, keep the legacy My Files space and use it for Photos.
pub fn roots(store: &Store, user: &User) -> Roots {
    if !store.spaces {
        return Roots { my_files: Root::Available(String::new()), photos: Root::Available(String::new()) };
    }
    if !valid_account_name(store, user) {
        return Roots { my_files: Root::Unavailable, photos: Root::Unavailable };
    }
    configured_roots(store, user).unwrap_or_else(|| resolve(store, user, &SavedRoots::default(), &BTreeMap::new()))
}

/// Existing accounts retain their default views until a library is explicitly selected.
pub fn configured_roots(store: &Store, user: &User) -> Option<Roots> {
    if !store.spaces || !valid_account_name(store, user) {
        return None;
    }
    let prefs = spaces::prefs(store, &user.username);
    let selection = prefs.get(SELECTION)?;
    let choices = prefs.get(KEY).map_or_else(BTreeMap::new, |key| choices_with(store, user, Some(key), &mount_table()));
    Some(resolve_saved(store, user, selection, &choices))
}

pub fn configured_photos(store: &Store, user: &User) -> Option<Root> {
    configured_roots(store, user).map(|roots| roots.photos)
}

/// Face metadata has its own identity even when rendered thumbnail bytes are shared.
pub fn face_model(store: &Store, user: Option<&User>, model: &str) -> String {
    use sha2::{Digest as _, Sha256};

    let Some(user) = user else { return model.to_string() };
    let Some(root) = configured_photos(store, user) else { return model.to_string() };
    let prefs = spaces::prefs(store, &user.username);
    let Some(key) = prefs.get(KEY).and_then(Value::as_str).filter(|key| valid_key(key)) else {
        return format!("{model}:unavailable");
    };
    let Root::Available(root) = root else { return format!("{model}:unavailable") };
    let scope = Sha256::digest(format!("{key}\0{}\0{root}", user.username));
    format!("{model}:{}", hex::encode(scope))
}

fn resolve_saved(store: &Store, user: &User, selection: &Value, choices: &BTreeMap<String, String>) -> Roots {
    let Ok(saved) = serde_json::from_value::<SavedRoots>(selection.clone()) else {
        return Roots { my_files: Root::Unavailable, photos: Root::Unavailable };
    };
    resolve(store, user, &saved, choices)
}

/// Replace both settings atomically under the server's store lock. None means the legacy
/// space for My Files and the effective My Files root for Photos.
pub fn set_roots(store: &Store, user: &User, my_files: Option<&str>, photos: Option<PhotosChoice>) -> io::Result<Roots> {
    check_account(store, user)?;
    set_with(store, user, my_files, photos, &mount_table())
}

fn set_with(store: &Store, user: &User, my_files: Option<&str>, photos: Option<PhotosChoice>, table: &MountTable) -> io::Result<Roots> {
    let mut prefs = spaces::prefs(store, &user.username);
    let choices = choices_with(store, user, prefs.get(KEY), table);
    let assigned = |id: &str| -> io::Result<Assignment> {
        choices
            .get(id)
            .map(|path| Assignment { id: id.to_string(), path: path.clone() })
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "That mount is not assigned or is unavailable."))
    };
    let my_files = my_files.map(assigned).transpose()?;
    let photos = match photos {
        Some(PhotosChoice::Mount(id)) => Some(SavedPhotos::Mount(assigned(&id)?)),
        Some(PhotosChoice::Folder(path)) => Some(SavedPhotos::Folder(path)),
        None => None,
    };
    let saved = SavedRoots { my_files, photos };
    let resolved = resolve(store, user, &saved, &choices);
    if resolved.my_files == Root::Unavailable || resolved.photos == Root::Unavailable {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Choose an existing Photos folder inside My Files or an assigned mount."));
    }
    if saved.my_files.is_none() && saved.photos.is_none() {
        prefs.remove(SELECTION);
    } else {
        if !prefs.get(KEY).and_then(Value::as_str).is_some_and(valid_key) {
            prefs.insert(KEY.to_string(), json!(store::random_hex(16)));
        }
        prefs.insert(SELECTION.to_string(), json!(saved));
    }
    spaces::save_prefs(store, &user.username, &prefs)?;
    Ok(resolved)
}

fn resolve(store: &Store, user: &User, saved: &SavedRoots, choices: &BTreeMap<String, String>) -> Roots {
    let mounted = |a: &Assignment| {
        if choices.get(&a.id) == Some(&a.path) { Root::Available(a.path.clone()) } else { Root::Unavailable }
    };
    let my_files = saved.my_files.as_ref().map_or_else(|| Root::Available(user.username.clone()), mounted);
    let photos = match &saved.photos {
        None => my_files.clone(),
        Some(SavedPhotos::Mount(assignment)) => mounted(assignment),
        Some(SavedPhotos::Folder(folder)) => {
            if let Root::Available(base) = &my_files
                && is_relative(folder)
            {
                let path = store::join(base, folder);
                if (!private_path(&path) || private_path(base)) && safe_folder(store, &path) { Root::Available(path) } else { Root::Unavailable }
            } else {
                Root::Unavailable
            }
        }
    };
    Roots { my_files, photos }
}

fn valid_account_name(store: &Store, user: &User) -> bool {
    store.spaces && auth::clean_username(&user.username).is_ok_and(|name| name == user.username)
}

fn check_account(store: &Store, user: &User) -> io::Result<()> {
    if valid_account_name(store, user) && auth::users(store).iter().any(|active| active.username == user.username) {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "This account cannot configure mounts."))
    }
}

fn valid_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_relative(path: &str) -> bool {
    !path.is_empty() && !path.chars().any(char::is_control) && store::rel(Some(path)).is_ok_and(|rel| rel == path)
}

fn safe_folder(store: &Store, path: &str) -> bool {
    store.no_links(path) && fs::symlink_metadata(store.full(path)).is_ok_and(|m| m.is_dir())
}

fn choices_with(store: &Store, user: &User, key: Option<&Value>, table: &MountTable) -> BTreeMap<String, String> {
    let Some(key) = key.and_then(Value::as_str).filter(|key| valid_key(key)) else { return BTreeMap::new() };
    let grants = store.load_json(&store.meta().join("mounts.json"));
    let Some(mine) = grants.get(key).and_then(Value::as_object) else { return BTreeMap::new() };
    let Ok(root) = fs::canonicalize(&store.root) else { return BTreeMap::new() };
    let mount_at = |path: &str| table.get(&root.join(path)).and_then(Option::as_ref);
    let elsewhere: Vec<&Mount> = grants
        .iter()
        .filter(|(other, _)| *other != key)
        .filter_map(|(_, entries)| entries.as_object())
        .flat_map(|entries| entries.values().filter_map(Value::as_str))
        .filter(|path| is_relative(path))
        .filter_map(mount_at)
        .collect();
    mine.iter()
        .filter_map(|(id, path)| {
            let path = path.as_str()?;
            if !valid_id(id)
                || !is_relative(path)
                || !private_path(path)
                || path.split('/').count() != 3
                || store::space_of(path) != user.username
                || !safe_folder(store, path)
            {
                return None;
            }
            let target = root.join(path);
            let mount = table.get(&target)?.as_ref()?;
            if table.keys().any(|other| other != &target && other.starts_with(&target)) {
                return None;
            }
            let overlaps =
                |other: &&Mount| other.device == mount.device && (other.source.starts_with(&mount.source) || mount.source.starts_with(&other.source));
            (!elsewhere.iter().any(overlaps)).then(|| (id.clone(), path.to_string()))
        })
        .collect()
}

fn mount_table() -> MountTable {
    fs::read("/proc/self/mountinfo").map_or_else(|_| HashMap::new(), |info| parse_mount_table(&info))
}

fn parse_mount_table(info: &[u8]) -> MountTable {
    let mut mounts = HashMap::new();
    for line in info.split(|b| *b == b'\n') {
        let mut fields = line.split(|b| *b == b' ').skip(2);
        let (Some(device), Some(source), Some(target)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let (Some(source), Some(target)) = (mount_path(source), mount_path(target)) else { continue };
        mounts.entry(target).and_modify(|stack| *stack = None).or_insert_with(|| Some(Mount { device: device.to_vec(), source }));
    }
    mounts
}

fn mount_path(path: &[u8]) -> Option<PathBuf> {
    let mut rest = path;
    let mut bytes = Vec::with_capacity(path.len());
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'\\' {
            let [a @ b'0'..=b'3', b @ b'0'..=b'7', c @ b'0'..=b'7', tail @ ..] = tail else { return None };
            bytes.push((a - b'0') * 64 + (b - b'0') * 8 + (c - b'0'));
            rest = tail;
        } else {
            bytes.push(first);
            rest = tail;
        }
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    struct Fixture {
        store: Store,
        ann: User,
        bob: User,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("starbase-mounts-{}", store::random_hex(6)));
            fs::create_dir_all(&root).unwrap();
            let store = Store { root, keep: 2, spaces: true };
            let ann = auth::add_hashed(&store, "ann", "Ann", String::new()).unwrap();
            let bob = auth::add_hashed(&store, "bob", "Bob", String::new()).unwrap();
            Self { store, ann, bob }
        }

        fn directory(&self, path: &str) {
            fs::create_dir_all(self.store.full(path)).unwrap();
        }

        fn provision(&self, grants: Value) {
            self.store.save_json(&self.store.meta().join("mounts.json"), &grants).unwrap();
        }

        fn table(&self, targets: &[(&str, &str)]) -> MountTable {
            let root = fs::canonicalize(&self.store.root).unwrap();
            targets
                .iter()
                .map(|(target, source)| (root.join(target), Some(Mount { device: b"8:1".to_vec(), source: PathBuf::from(source) })))
                .collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.store.root);
        }
    }

    #[test]
    fn private_paths_without_live_grants_stay_hidden_even_from_admin_and_public_links() {
        use crate::sharing::{Access, Role};

        let f = Fixture::new();
        let folder = "bob/starbase-mounts/Library";
        f.directory(folder);
        let path = store::join(folder, "Photo.png");
        fs::write(f.store.full(&path), b"fixture").unwrap();
        let id = f.store.synced_index().unwrap().into_iter().find(|(_, p)| p == &path).unwrap().0;

        for (auth, user) in [(true, Some(&f.ann)), (true, Some(&f.bob)), (false, None)] {
            let access = Access::new(&f.store, auth, user);
            assert_eq!(access.file_role(&id), Role::None);
            assert_eq!(access.folder_role(folder), Role::None);
            assert!(!access.file_listed(&id));
            assert!(!access.folder_listed(folder));
            assert!(!access.folder_ids.contains_key(folder));
        }

        crate::publiclink::update(&f.store, &id, &json!({"public": {"on": true, "password": "test fixture"}})).unwrap();
        let link = crate::publiclink::describe(&f.store, &id);
        let token = link["token"].as_str().unwrap();
        assert_eq!(crate::publiclink::serve(&f.store, token, "download", "").code, 404);
        assert_eq!(crate::publiclink::unlock(&f.store, token, "test fixture").code, 404);

        let no_auth = Store { root: f.store.root.clone(), keep: 2, spaces: false };
        assert_eq!(no_auth.synced_index().unwrap().get(&id), Some(&path));
        assert!(no_auth.path_of(&id).unwrap().is_none());
        assert!(!no_auth.no_links(folder));
        assert_eq!(crate::publiclink::serve(&no_auth, token, "download", "").code, 404);
    }

    #[test]
    fn explicit_photo_scope_does_not_restrict_drive_and_fails_closed_when_lost() {
        use crate::sharing::{Access, Role};

        let f = Fixture::new();
        f.directory("ann/Library");
        fs::write(f.store.full("ann/Outside.png"), b"fixture").unwrap();
        let id = f.store.synced_index().unwrap().into_iter().find(|(_, p)| p == "ann/Outside.png").unwrap().0;
        let default = Access::new(&f.store, true, Some(&f.ann));
        assert!(default.photo_allowed(&f.store, "ann/Outside.png"));
        assert!(!default.photo_allowed(&f.store, "ann/starbase-mounts/Old Library/Photo.png"));

        set_with(&f.store, &f.ann, None, Some(PhotosChoice::Folder("Library".into())), &MountTable::new()).unwrap();
        let scoped = Access::new(&f.store, true, Some(&f.ann));
        assert!(scoped.photo_allowed(&f.store, "ann/Library/Inside.png"));
        assert!(!scoped.photo_allowed(&f.store, "ann/Library Other/Outside.png"));
        assert!(!scoped.photo_allowed(&f.store, "ann/Outside.png"));
        assert_eq!(scoped.file_role(&id), Role::Owner);

        fs::remove_dir(f.store.full("ann/Library")).unwrap();
        let lost = Access::new(&f.store, true, Some(&f.ann));
        assert!(!lost.photo_allowed(&f.store, "ann/Outside.png"));
        assert_eq!(lost.file_role(&id), Role::Owner);
    }

    #[test]
    fn no_settings_means_legacy_roots_without_provisioning() {
        let f = Fixture::new();
        assert_eq!(roots(&f.store, &f.ann), Roots { my_files: Root::Available("ann".into()), photos: Root::Available("ann".into()) });
        assert!(choices(&f.store, &f.ann).is_empty());
        assert!(spaces::prefs(&f.store, "ann").get(KEY).is_none());
    }

    #[test]
    fn only_assigned_live_mounts_in_the_account_space_are_offered() {
        let f = Fixture::new();
        let ann = account_key(&f.store, &f.ann).unwrap();
        let bob = account_key(&f.store, &f.bob).unwrap();
        assert_ne!(ann, bob);
        f.provision(json!({
            (ann): {
                "valid": "ann/starbase-mounts/Media", "wrong-account": "bob/starbase-mounts/BobFiles", "missing": "ann/starbase-mounts/Unplugged",
                "traversal": "ann/../bob", "absolute": "/ann/starbase-mounts/Media", "symbolic": "ann/starbase-mounts/Alias",
                "duplicate": "ann/starbase-mounts/Overlap", "prefix": "ann-other/Files",
                "account-root": "ann", "ordinary": "ann/Ordinary", "nested": "ann/starbase-mounts/Nested"
            },
            (bob): {"files": "bob/starbase-mounts/BobFiles"}
        }));
        for path in [
            "ann/starbase-mounts/Media",
            "ann/starbase-mounts/Unplugged",
            "ann/starbase-mounts/Overlap",
            "bob/starbase-mounts/BobFiles",
            "ann-other/Files",
            "ann/starbase-mounts/Nested",
            "ann/Ordinary",
        ] {
            f.directory(path);
        }
        symlink(f.store.full("ann/starbase-mounts/Media"), f.store.full("ann/starbase-mounts/Alias")).unwrap();
        let table = f.table(&[
            ("ann/starbase-mounts/Media", "/sources/media"),
            ("ann/starbase-mounts/Alias", "/sources/alias"),
            ("ann/starbase-mounts/Overlap", "/sources/bob/sub"),
            ("bob/starbase-mounts/BobFiles", "/sources/bob"),
            ("ann-other/Files", "/sources/other"),
            ("ann", "/sources/account"),
            ("ann/Ordinary", "/sources/ordinary"),
            ("ann/starbase-mounts/Nested", "/sources/nested"),
            ("ann/starbase-mounts/Nested/Child", "/sources/bob"),
        ]);
        let prefs = spaces::prefs(&f.store, "ann");
        assert_eq!(choices_with(&f.store, &f.ann, prefs.get(KEY), &table), BTreeMap::from([("valid".into(), "ann/starbase-mounts/Media".into())]));
        assert!(set_with(&f.store, &f.ann, Some("wrong-account"), None, &table).is_err());
        assert!(spaces::prefs(&f.store, "ann").get(SELECTION).is_none());
    }

    #[test]
    fn roots_are_saved_together_and_a_revoked_choice_never_falls_back() {
        let f = Fixture::new();
        let key = account_key(&f.store, &f.ann).unwrap();
        f.provision(json!({(key.as_str()): {"files": "ann/starbase-mounts/Files", "photos": "ann/starbase-mounts/Separate Photos"}}));
        for path in ["ann/starbase-mounts/Files/Family", "ann/starbase-mounts/Separate Photos", "bob/Family", "ann/starbase-mounts/Different Files"] {
            f.directory(path);
        }
        symlink(f.store.full("bob/Family"), f.store.full("ann/starbase-mounts/Files/Link")).unwrap();
        let table = f.table(&[
            ("ann/starbase-mounts/Files", "/files"),
            ("ann/starbase-mounts/Separate Photos", "/photos"),
            ("ann/starbase-mounts/Different Files", "/different"),
        ]);
        let my_files = Root::Available("ann/starbase-mounts/Files".into());
        assert_eq!(
            set_with(&f.store, &f.ann, Some("files"), Some(PhotosChoice::Folder("Family".into())), &table).unwrap(),
            Roots { my_files: my_files.clone(), photos: Root::Available("ann/starbase-mounts/Files/Family".into()) }
        );
        let before = spaces::prefs(&f.store, "ann");
        for path in ["../Separate Photos", "/bob/Family", "Link", "Missing"] {
            assert!(set_with(&f.store, &f.ann, Some("files"), Some(PhotosChoice::Folder(path.into())), &table).is_err(), "{path}");
            assert_eq!(spaces::prefs(&f.store, "ann"), before);
        }
        let separate = Some(PhotosChoice::Mount("photos".into()));
        let expected = Roots { my_files, photos: Root::Available("ann/starbase-mounts/Separate Photos".into()) };
        assert_eq!(set_with(&f.store, &f.ann, Some("files"), separate, &table).unwrap(), expected);

        let prefs = spaces::prefs(&f.store, "ann");
        let selected = &prefs[SELECTION];
        let available = choices_with(&f.store, &f.ann, prefs.get(KEY), &table);
        assert_eq!(resolve_saved(&f.store, &f.ann, selected, &available), expected);
        assert_eq!(roots(&f.store, &f.ann), Roots { my_files: Root::Unavailable, photos: Root::Unavailable }, "directories alone are insufficient");
        f.provision(json!({(key): {"files": "ann/starbase-mounts/Different Files", "photos": "ann/starbase-mounts/Separate Photos"}}));
        let changed = choices_with(&f.store, &f.ann, prefs.get(KEY), &table);
        assert_eq!(
            resolve_saved(&f.store, &f.ann, selected, &changed),
            Roots { my_files: Root::Unavailable, photos: Root::Available("ann/starbase-mounts/Separate Photos".into()) }
        );
    }

    #[test]
    fn a_recreated_account_cannot_inherit_an_old_mount_grant() {
        let f = Fixture::new();
        let key = account_key(&f.store, &f.bob).unwrap();
        f.provision(json!({(key): {"files": "bob/starbase-mounts/Files"}}));
        auth::remove(&f.store, "bob").unwrap();
        spaces::retire(&f.store, "bob").unwrap();
        let bob = auth::add_hashed(&f.store, "bob", "New Bob", String::new()).unwrap();
        let key = spaces::prefs(&f.store, "bob").remove(KEY);
        assert!(key.is_none());
        assert!(choices_with(&f.store, &bob, key.as_ref(), &f.table(&[("bob/starbase-mounts/Files", "/private")])).is_empty());
    }

    #[test]
    fn mountinfo_decodes_names_and_rejects_ambiguous_mountpoints() {
        let table = parse_mount_table(
            b"42 1 8:1 /from\\040here /data/ann/My\\040Files rw - ext4 /dev/root rw\n\
              43 1 8:2 /one /data/bob/Mount rw - ext4 /dev/one rw\n\
              44 1 8:3 /two /data/bob/Mount rw - ext4 /dev/two rw\n",
        );
        assert_eq!(table[Path::new("/data/ann/My Files")].as_ref().unwrap().source, Path::new("/from here"));
        assert!(table[Path::new("/data/bob/Mount")].is_none());
    }
}
