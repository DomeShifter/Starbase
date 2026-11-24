//! Accounts (live editing step 2): people sign in with a username and password kept on this
//! server (Argon2 hashes in STORE/.starsheets/users.json). No sign-in through other companies.
//!
//! The first account is the owner's. It is made in the app with a setup code from the server's
//! log (or STARSHEETS_PASSWORD when set), so nobody else can claim a new server first. The owner
//! adds other people, or lets them make their own account (sign-up, off unless the owner turns
//! it on, optionally with a code). Everyone can change their own name, colour and password.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use serde_json::{Map, Value, json};

use crate::store::{Store, now, random_hex};

pub const SESSION_DAYS: u64 = 30;

/// Colours given to people in turn (cursors and avatars), readable on white. No blues: blue is
/// how your own selection looks.
pub const COLOURS: [&str; 8] = ["#D93025", "#188038", "#E37400", "#9334E6", "#007B83", "#E52592", "#B06000", "#5F6368"];

#[derive(Clone, Debug, PartialEq)]
pub struct User {
    pub username: String,
    pub name: String,
    pub colour: String,
    pub owner: bool,
}

impl User {
    pub fn json(&self) -> Value {
        json!({ "username": self.username, "name": self.name, "colour": self.colour, "owner": self.owner })
    }
}

fn users_path(store: &Store) -> std::path::PathBuf {
    store.meta().join("users.json")
}

fn sessions_path(store: &Store) -> std::path::PathBuf {
    store.meta().join("sessions.json")
}

fn load(store: &Store) -> Vec<Map<String, Value>> {
    store
        .load_json(&users_path(store))
        .get("users")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|u| u.as_object().cloned())
        .collect()
}

fn save(store: &Store, users: &[Map<String, Value>]) -> std::io::Result<()> {
    let mut all = store.load_json(&users_path(store));
    all.insert("users".into(), json!(users));
    store.save_json(&users_path(store), &Value::Object(all))
}

// ---------------------------------------------------------------- sign-up

/// Whether people may make their own account, and the code they need ("" = none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signup {
    pub on: bool,
    pub code: String,
}

pub fn signup(store: &Store) -> Signup {
    let all = store.load_json(&users_path(store));
    let s = all.get("signup");
    Signup {
        on: s.and_then(|s| s.get("on")).and_then(Value::as_bool).unwrap_or(false),
        code: s.and_then(|s| s.get("code")).and_then(Value::as_str).unwrap_or("").to_string(),
    }
}

pub fn set_signup(store: &Store, on: bool, code: &str) -> Result<Signup, String> {
    let code: String = code.trim().chars().filter(|c| !c.is_control()).take(64).collect();
    let mut all = store.load_json(&users_path(store));
    all.insert("signup".into(), json!({ "on": on, "code": code }));
    store.save_json(&users_path(store), &Value::Object(all)).map_err(|e| e.to_string())?;
    Ok(Signup { on, code })
}

/// Whether sign-up may make an account now, with this code: only when the owner allows it
/// (never the owner's account).
pub fn signup_allowed(store: &Store, code: &str) -> Result<(), (u16, String)> {
    let s = signup(store);
    if !any_users(store) || !s.on {
        return Err((403, "This server doesn't let people make their own account. Ask the person who runs it to add you.".into()));
    }
    if !s.code.is_empty() && !crate::same(code.trim().as_bytes(), s.code.as_bytes()) {
        return Err((401, "That sign-up code isn't right.".into()));
    }
    Ok(())
}

/// Makes an account through sign-up (signup_allowed, then add).
#[cfg(test)]
pub fn sign_up(store: &Store, username: &str, name: &str, password: &str, code: &str) -> Result<User, (u16, String)> {
    signup_allowed(store, code)?;
    add(store, username, name, password).map_err(|e| (400, e))
}

fn to_user(u: &Map<String, Value>) -> User {
    let s = |k: &str| u.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    User { username: s("username"), name: s("name"), colour: s("colour"), owner: u.get("owner").and_then(Value::as_bool).unwrap_or(false) }
}

pub fn users(store: &Store) -> Vec<User> {
    load(store).iter().map(to_user).collect()
}

pub fn any_users(store: &Store) -> bool {
    !load(store).is_empty()
}

// ---------------------------------------------------------------- passwords

pub fn hash(password: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    rand::fill(&mut salt[..]);
    let h: PasswordHash = Argon2::default().hash_password_with_salt(password.as_bytes(), &salt).map_err(|e| e.to_string())?;
    Ok(h.to_string())
}

/// A password's hash for a new or changed password (checks it first). Slow on purpose (Argon2):
/// call it outside the store lock.
pub fn hash_new(password: &str) -> Result<String, String> {
    good_password(password)?;
    hash(password)
}

/// Checks `password` against a stored hash; with no hash (no such person), against a made-up one,
/// so a wrong username takes as long as a wrong password (nobody can find usernames by timing).
/// Slow on purpose (Argon2): call it outside the store lock.
pub fn verify(password: &str, stored: Option<&str>) -> bool {
    static DUMMY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| hash(&random_hex(16)).unwrap_or_default());
    let too_long = password.len() > MAX_PASSWORD;
    let given = if too_long { "" } else { password };
    let ok = match stored.and_then(|s| PasswordHash::new(s).ok()) {
        Some(h) => Argon2::default().verify_password(given.as_bytes(), &h).is_ok(),
        None => {
            let _ = PasswordHash::new(&DUMMY).map(|h| Argon2::default().verify_password(given.as_bytes(), &h));
            false
        }
    };
    ok && !too_long && stored.is_some()
}

/// Someone's account and password hash, for checking a password outside the store lock.
pub fn login_hash(store: &Store, username: &str) -> Option<(User, String)> {
    let username = username.trim().to_lowercase();
    let users = load(store);
    let u = users.iter().find(|u| u.get("username").and_then(Value::as_str) == Some(username.as_str()))?;
    Some((to_user(u), u.get("hash").and_then(Value::as_str)?.to_string()))
}

/// Checks a username and password (slow on purpose: Argon2).
#[cfg(test)]
pub fn check(store: &Store, username: &str, password: &str) -> Option<User> {
    let found = login_hash(store, username);
    if verify(password, found.as_ref().map(|(_, h)| h.as_str())) { found.map(|(u, _)| u) } else { None }
}

// ---------------------------------------------------------------- checks on what people type

pub fn clean_username(s: &str) -> Result<String, String> {
    let u = s.trim().to_lowercase();
    if !(2..=32).contains(&u.chars().count()) || !u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) || u.starts_with('.') {
        return Err("A username is 2 to 32 letters, numbers, dots, dashes or underscores.".into());
    }
    Ok(u)
}

pub fn clean_display(s: &str) -> Result<String, String> {
    let n: String = s.trim().chars().filter(|c| !c.is_control()).take(60).collect();
    if n.is_empty() { Err("Please give a name.".into()) } else { Ok(n) }
}

/// The longest password accepted, in bytes (Argon2 work grows with it).
pub const MAX_PASSWORD: usize = 1024;

pub fn good_password(p: &str) -> Result<(), String> {
    if p.chars().count() < 8 {
        Err("Use at least 8 characters for the password.".into())
    } else if p.len() > MAX_PASSWORD {
        Err(format!("A password can be at most {MAX_PASSWORD} characters."))
    } else {
        Ok(())
    }
}

fn clean_colour(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_uppercase())
}

// ---------------------------------------------------------------- changing accounts

/// Adds an account (the first one is the owner's).
#[cfg(test)]
pub fn add(store: &Store, username: &str, name: &str, password: &str) -> Result<User, String> {
    check_new(store, username, name)?;
    add_hashed(store, username, name, hash_new(password)?)
}

/// Checks a new account's username and name before the (slow) password hash is made.
pub fn check_new(store: &Store, username: &str, name: &str) -> Result<(), String> {
    let username = clean_username(username)?;
    clean_display(name)?;
    if load(store).iter().any(|u| u.get("username").and_then(Value::as_str) == Some(username.as_str())) {
        return Err("That username is taken.".into());
    }
    Ok(())
}

/// Adds an account with a password hash from hash_new (the first account is the owner's).
pub fn add_hashed(store: &Store, username: &str, name: &str, hash: String) -> Result<User, String> {
    let username = clean_username(username)?;
    let name = clean_display(name)?;
    let mut users = load(store);
    if users.iter().any(|u| u.get("username").and_then(Value::as_str) == Some(username.as_str())) {
        return Err("That username is taken.".into());
    }
    let owner = users.is_empty();
    let colour = COLOURS[users.len() % COLOURS.len()].to_string();
    let mut u = Map::new();
    u.insert("username".into(), json!(username));
    u.insert("name".into(), json!(name));
    u.insert("colour".into(), json!(colour));
    u.insert("owner".into(), json!(owner));
    u.insert("hash".into(), json!(hash));
    u.insert("created".into(), json!(now() as i64));
    let user = to_user(&u);
    users.push(u);
    save(store, &users).map_err(|e| e.to_string())?;
    Ok(user)
}

/// Changes someone's name, colour or password (`None` leaves it).
#[cfg(test)]
pub fn update(store: &Store, username: &str, name: Option<&str>, colour: Option<&str>, password: Option<&str>) -> Result<User, String> {
    update_hashed(store, username, name, colour, password.map(hash_new).transpose()?)
}

/// Changes someone's name, colour or password hash (from hash_new; `None` leaves it). A new
/// password signs them out everywhere.
pub fn update_hashed(store: &Store, username: &str, name: Option<&str>, colour: Option<&str>, hash: Option<String>) -> Result<User, String> {
    let mut users = load(store);
    let u = users.iter_mut().find(|u| u.get("username").and_then(Value::as_str) == Some(username)).ok_or("No such person.")?;
    if let Some(n) = name {
        u.insert("name".into(), json!(clean_display(n)?));
    }
    if let Some(c) = colour {
        u.insert("colour".into(), json!(clean_colour(c).ok_or("That isn't a colour.")?));
    }
    let new_password = hash.is_some();
    if let Some(h) = hash {
        u.insert("hash".into(), json!(h));
    }
    let user = to_user(u);
    save(store, &users).map_err(|e| e.to_string())?;
    if new_password {
        end_sessions(store, username);
    }
    Ok(user)
}

pub fn remove(store: &Store, username: &str) -> Result<(), String> {
    let mut users = load(store);
    let before = users.len();
    users.retain(|u| u.get("username").and_then(Value::as_str) != Some(username) || u.get("owner").and_then(Value::as_bool) == Some(true));
    if users.len() == before {
        return Err("That person can't be removed.".into());
    }
    save(store, &users).map_err(|e| e.to_string())?;
    end_sessions(store, username);
    Ok(())
}

// ---------------------------------------------------------------- sessions

fn token_hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// A new sign-in for `username`: the cookie value.
pub fn new_session(store: &Store, username: &str) -> std::io::Result<String> {
    let token = random_hex(32);
    let t = now();
    let mut sessions = store.load_json(&sessions_path(store));
    // Sessions from before accounts (a bare expiry time) and expired ones go.
    sessions.retain(|_, v| v.get("expires").and_then(Value::as_f64).unwrap_or(0.0) > t);
    sessions.insert(token_hash(&token), json!({ "user": username, "expires": t + (SESSION_DAYS * 86400) as f64 }));
    store.save_json(&sessions_path(store), &Value::Object(sessions))?;
    Ok(token)
}

pub fn end_session(store: &Store, token: &str) {
    let mut sessions = store.load_json(&sessions_path(store));
    if sessions.remove(&token_hash(token)).is_some() {
        let _ = store.save_json(&sessions_path(store), &Value::Object(sessions));
    }
}

fn end_sessions(store: &Store, username: &str) {
    let mut sessions = store.load_json(&sessions_path(store));
    sessions.retain(|_, v| v.get("user").and_then(Value::as_str) != Some(username));
    let _ = store.save_json(&sessions_path(store), &Value::Object(sessions));
}

/// Who a session cookie belongs to.
pub fn session_user(store: &Store, token: &str) -> Option<User> {
    let sessions = store.load_json(&sessions_path(store));
    let s = sessions.get(&token_hash(token))?;
    if s.get("expires").and_then(Value::as_f64).unwrap_or(0.0) <= now() {
        return None;
    }
    let username = s.get("user").and_then(Value::as_str)?;
    users(store).into_iter().find(|u| u.username == username)
}

// ---------------------------------------------------------------- the setup code

/// The shortest setup code accepted in STARSHEETS_PASSWORD.
pub const MIN_SETUP_CODE: usize = 12;

/// Why a setup code given in STARSHEETS_PASSWORD is too easy to guess (None: it's fine). The
/// server refuses to start with one, so nobody else can claim a new server first.
pub fn weak_setup_code(code: &str) -> Option<String> {
    let code = code.trim();
    if code.eq_ignore_ascii_case("change-me") {
        Some("STARSHEETS_PASSWORD is still \"change-me\", the example from the instructions. Remove it (a random setup code is then printed in the log) or set your own long one.".into())
    } else if code.chars().count() < MIN_SETUP_CODE {
        Some(format!(
            "STARSHEETS_PASSWORD is too short to be a safe setup code: use at least {MIN_SETUP_CODE} characters, or remove it (a random setup code is then printed in the log)."
        ))
    } else {
        None
    }
}

/// The setup code given in STARSHEETS_PASSWORD, if any.
pub fn given_setup_code() -> Option<String> {
    std::env::var("STARSHEETS_PASSWORD").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The code that makes the owner account on a new server: STARSHEETS_PASSWORD when set, else one
/// made on first start and kept in STORE/.starsheets/setup-code.
pub fn setup_code(store: &Store) -> String {
    if let Some(given) = given_setup_code() {
        return given;
    }
    let path = store.meta().join("setup-code");
    if let Ok(s) = std::fs::read_to_string(&path) {
        return s.trim().to_string();
    }
    let made = random_hex(MIN_SETUP_CODE / 2);
    let _ = std::fs::create_dir_all(store.meta());
    let _ = std::fs::write(&path, format!("{made}\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("starsheets-auth-{}", random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        Store { root: dir, keep: 2, spaces: false }
    }

    #[test]
    fn accounts_and_sessions() {
        let s = store();
        assert!(add(&s, "x", "X", "longenough").is_err(), "short username");
        assert!(add(&s, "james", "James", "short").is_err(), "short password");
        let owner = add(&s, " James ", "James", "correct horse").unwrap();
        assert!(owner.owner && owner.username == "james");
        let other = add(&s, "aurelie", "Aurélie", "another one").unwrap();
        assert!(!other.owner && other.colour != owner.colour);
        assert!(add(&s, "JAMES", "Again", "whatever12").is_err(), "taken");
        assert!(check(&s, "james", "wrong password").is_none());
        assert_eq!(check(&s, "James", "correct horse").unwrap().username, "james");
        let token = new_session(&s, "aurelie").unwrap();
        assert_eq!(session_user(&s, &token).unwrap().name, "Aurélie");
        update(&s, "aurelie", Some("A."), Some("#00ff00"), None).unwrap();
        assert_eq!(session_user(&s, &token).unwrap().colour, "#00FF00");
        update(&s, "aurelie", None, None, Some("a new password")).unwrap();
        assert!(session_user(&s, &token).is_none(), "a new password signs out everywhere");
        assert!(remove(&s, "james").is_err(), "the owner stays");
        remove(&s, "aurelie").unwrap();
        assert_eq!(users(&s).len(), 1);
        // Sign-up: off by default, the code when one is set, never touching the people list.
        assert_eq!(sign_up(&s, "newbie", "New", "long enough", "").unwrap_err().0, 403);
        set_signup(&s, true, " letmein ").unwrap();
        assert_eq!(sign_up(&s, "newbie", "New", "long enough", "wrong").unwrap_err().0, 401);
        let n = sign_up(&s, "newbie", "New", "long enough", "letmein").unwrap();
        assert!(!n.owner);
        assert_eq!(signup(&s), Signup { on: true, code: "letmein".into() });
        set_signup(&s, true, "").unwrap();
        sign_up(&s, "second", "Two", "long enough", "").unwrap();
        assert_eq!(users(&s).len(), 3, "saving people keeps the sign-up setting and vice versa");
        assert!(signup(&s).on);
        // Passwords: at most MAX_PASSWORD bytes; an unknown username is simply wrong.
        let long = "x".repeat(MAX_PASSWORD + 1);
        assert!(add(&s, "longpw", "Long", &long).is_err());
        assert!(update(&s, "second", None, None, Some(&long)).is_err());
        assert!(check(&s, "second", &long).is_none());
        assert!(check(&s, "nobody", "long enough").is_none());
        assert!(!verify("long enough", None) && !verify("long enough", Some("not a hash")));
        let _ = std::fs::remove_dir_all(&s.root);
    }

    #[test]
    fn setup_codes_that_are_too_easy() {
        assert!(weak_setup_code("change-me").is_some());
        assert!(weak_setup_code(" CHANGE-ME ").is_some());
        assert!(weak_setup_code("short-code1").is_some(), "11 characters");
        assert!(weak_setup_code("a-long-enough-code").is_none());
        let s = store();
        assert_eq!(setup_code(&s).len(), MIN_SETUP_CODE, "a made-up code is long enough too");
        assert!(weak_setup_code(&setup_code(&s)).is_none());
        let _ = std::fs::remove_dir_all(&s.root);
    }
}
