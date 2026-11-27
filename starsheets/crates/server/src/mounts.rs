//! Resolve My Files and Photos settings against administrator-provisioned bind mounts.
//!
//! The deployment administrator mounts a directory under `Store::root`, inside the
//! target user's space, and records it in `.starsheets/mounts.json`. For example:
//! `{"<account key>": {"library": "ann/Photo Library"}}`. The account key is stored in
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
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::{self, User};
use crate::spaces;
use crate::store::{self, Store};

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

/// Get or create the provisioning key only when the administrator sets up mounts.
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
    choices_with(store, user, prefs.get(KEY), &mount_table())
}

/// Read current roots. With no preference, keep the legacy My Files space and use it for Photos.
pub fn roots(store: &Store, user: &User) -> Roots {
    if !store.spaces {
        return Roots { my_files: Root::Available(String::new()), photos: Root::Available(String::new()) };
    }
    if !valid_account_name(store, user) {
        return Roots { my_files: Root::Unavailable, photos: Root::Unavailable };
    }
    let prefs = spaces::prefs(store, &user.username);
    let Some(selection) = prefs.get(SELECTION) else {
        return resolve(store, user, &SavedRoots::default(), &BTreeMap::new());
    };
    let table = if prefs.contains_key(KEY) { mount_table() } else { MountTable::new() };
    resolve_saved(store, user, selection, &choices_with(store, user, prefs.get(KEY), &table))
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
                if safe_folder(store, &path) { Root::Available(path) } else { Root::Unavailable }
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
            if !valid_id(id) || !is_relative(path) || store::space_of(path) != user.username || !safe_folder(store, path) {
                return None;
            }
            let mount = mount_at(path)?;
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
                "valid": "ann/Media", "wrong-account": "bob/BobFiles", "missing": "ann/Unplugged",
                "traversal": "ann/../bob", "absolute": "/ann/Media", "symbolic": "ann/Alias",
                "duplicate": "ann/Overlap", "prefix": "ann-other/Files"
            },
            (bob): {"files": "bob/BobFiles"}
        }));
        for path in ["ann/Media", "ann/Unplugged", "ann/Overlap", "bob/BobFiles", "ann-other/Files"] {
            f.directory(path);
        }
        symlink(f.store.full("ann/Media"), f.store.full("ann/Alias")).unwrap();
        let table = f.table(&[
            ("ann/Media", "/sources/media"), ("ann/Alias", "/sources/alias"), ("ann/Overlap", "/sources/bob/sub"),
            ("bob/BobFiles", "/sources/bob"), ("ann-other/Files", "/sources/other")
        ]);
        let prefs = spaces::prefs(&f.store, "ann");
        assert_eq!(choices_with(&f.store, &f.ann, prefs.get(KEY), &table), BTreeMap::from([("valid".into(), "ann/Media".into())]));
        assert!(set_with(&f.store, &f.ann, Some("wrong-account"), None, &table).is_err());
        assert!(spaces::prefs(&f.store, "ann").get(SELECTION).is_none());
    }

    #[test]
    fn roots_are_saved_together_and_a_revoked_choice_never_falls_back() {
        let f = Fixture::new();
        let key = account_key(&f.store, &f.ann).unwrap();
        f.provision(json!({(key.as_str()): {"files": "ann/Files", "photos": "ann/Separate Photos"}}));
        for path in ["ann/Files/Family", "ann/Separate Photos", "bob/Family", "ann/Different Files"] {
            f.directory(path);
        }
        symlink(f.store.full("bob/Family"), f.store.full("ann/Files/Link")).unwrap();
        let table = f.table(&[("ann/Files", "/files"), ("ann/Separate Photos", "/photos"), ("ann/Different Files", "/different")]);
        let my_files = Root::Available("ann/Files".into());
        assert_eq!(
            set_with(&f.store, &f.ann, Some("files"), Some(PhotosChoice::Folder("Family".into())), &table).unwrap(),
            Roots { my_files: my_files.clone(), photos: Root::Available("ann/Files/Family".into()) }
        );
        let before = spaces::prefs(&f.store, "ann");
        for path in ["../Separate Photos", "/bob/Family", "Link", "Missing"] {
            assert!(set_with(&f.store, &f.ann, Some("files"), Some(PhotosChoice::Folder(path.into())), &table).is_err(), "{path}");
            assert_eq!(spaces::prefs(&f.store, "ann"), before);
        }
        let separate = Some(PhotosChoice::Mount("photos".into()));
        let expected = Roots { my_files, photos: Root::Available("ann/Separate Photos".into()) };
        assert_eq!(set_with(&f.store, &f.ann, Some("files"), separate, &table).unwrap(), expected);

        let prefs = spaces::prefs(&f.store, "ann");
        let selected = &prefs[SELECTION];
        let available = choices_with(&f.store, &f.ann, prefs.get(KEY), &table);
        assert_eq!(resolve_saved(&f.store, &f.ann, selected, &available), expected);
        assert_eq!(roots(&f.store, &f.ann), Roots { my_files: Root::Unavailable, photos: Root::Unavailable }, "directories alone are insufficient");
        f.provision(json!({(key): {"files": "ann/Different Files", "photos": "ann/Separate Photos"}}));
        let changed = choices_with(&f.store, &f.ann, prefs.get(KEY), &table);
        assert_eq!(
            resolve_saved(&f.store, &f.ann, selected, &changed),
            Roots { my_files: Root::Unavailable, photos: Root::Available("ann/Separate Photos".into()) }
        );
    }

    #[test]
    fn a_recreated_account_cannot_inherit_an_old_mount_grant() {
        let f = Fixture::new();
        let key = account_key(&f.store, &f.bob).unwrap();
        f.provision(json!({(key): {"files": "bob/Files"}}));
        auth::remove(&f.store, "bob").unwrap();
        spaces::retire(&f.store, "bob").unwrap();
        let bob = auth::add_hashed(&f.store, "bob", "New Bob", String::new()).unwrap();
        let key = spaces::prefs(&f.store, "bob").remove(KEY);
        assert!(key.is_none());
        assert!(choices_with(&f.store, &bob, key.as_ref(), &f.table(&[("bob/Files", "/private")])).is_empty());
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
