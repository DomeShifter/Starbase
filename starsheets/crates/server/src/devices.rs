//! Device sign-in for the desktop apps ("Connect to a server…"), in the style of `gh auth login`:
//!
//! 1. The app sends `POST /api/devices/start {name}` and gets a `device_code` (secret, the app's
//!    own), a short `user_code` (`ABCD-EFGH`) and the address of the connect page.
//! 2. The person opens that page in their browser, signs in there if they need to (password or
//!    single sign-on, as for the web app), checks the code and the device's name, and allows it
//!    (`POST /api/devices/approve`: a signed-in browser session only, from this server's own page).
//! 3. Meanwhile the app asks `POST /api/devices/poll {device_code}` every few seconds; once allowed
//!    it gets a device token, once (the request is then gone).
//!
//! A device token is long and random; only its SHA-256 is kept (STORE/.starsheets/devices.json),
//! with the device's name, its account, when it was made and last used. The app sends it as
//! `Authorization: Bearer …` on the API and on the live WebSocket. It acts as its account, with
//! the same rights (never more): owner-only things only for an owner. Each one can be removed
//! from "Your devices" (or by the owner in People), and stops working at once, open live
//! connections too. A new password, or the account being removed, removes all of its devices.
//!
//! Requests waiting for approval are kept in memory only (10 minutes); a restart forgets them.

use std::sync::{LazyLock, Mutex};

use serde_json::{Map, Value, json};

use crate::store::{Store, now, random_hex};

/// How long a sign-in request waits for approval.
pub const CODE_SECS: f64 = 600.0;
/// How often the app should ask (seconds).
pub const INTERVAL: u64 = 5;
/// The letters of a user code: no vowels (no words), nothing easily mistaken (0/O, 1/I/L, 5/S,
/// 2/Z, 8/B, U/V).
const ALPHABET: &[u8] = b"CDFGHJKMNPQRTWXY34679";
/// Most requests waiting at once (all addresses; past it the oldest goes, so a flood from many
/// addresses can't stop everyone else signing in), and from one address (behind a reverse
/// proxy, set STARBASE_TRUSTED_PROXY so each person's own address counts, not the proxy's).
const MAX_WAITING: usize = 10_000;
const MAX_WAITING_PER_IP: usize = 10;
/// Devices unused this long are removed (days; STARBASE_DEVICE_IDLE_DAYS, 0 = never).
const IDLE_DAYS: f64 = 90.0;
/// The prefix of every device token (lets secret scanners spot one that leaked).
pub const PREFIX: &str = "sbd_";
/// "Last used" is written at most this often per device (seconds).
const TOUCH_EVERY: f64 = 60.0;

#[derive(Clone, Debug, PartialEq)]
enum State {
    Waiting,
    /// Allowed: the device token, made when it was allowed (so anything that removes the
    /// account's devices afterwards, a new password say, removes this one too).
    Allowed(String),
    Denied,
}

#[derive(Clone, Debug)]
struct Request {
    device_code: String,
    user_code: String,
    name: String,
    from: String,
    made: f64,
    state: State,
}

/// Sign-in requests waiting for approval (kept in memory).
#[derive(Default)]
pub struct Waiting {
    list: Mutex<Vec<Request>>,
}

/// Makes a device token from the device's name (when a request is allowed).
pub type Mint<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The server's waiting requests.
pub static WAITING: LazyLock<Waiting> = LazyLock::new(Waiting::default);

/// What the app hears when it asks.
#[derive(Debug, PartialEq)]
pub enum Poll {
    /// Not allowed (or denied) yet.
    Waiting,
    /// Allowed: the device token (handed out once; the caller checks it still works).
    Allowed(String),
    Denied,
    Expired,
    /// No such request (never made, already answered, or forgotten by a restart).
    Unknown,
}

/// A user code as typed: upper case, without the dash or spaces.
pub fn clean_code(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

fn new_user_code() -> String {
    // 256 isn't a multiple of the alphabet: bytes in the uneven top are skipped, so every letter
    // is as likely.
    let n = ALPHABET.len() as u16;
    let limit = 256 - 256 % n;
    let mut out = String::new();
    while out.len() < 8 {
        let mut b = [0u8; 1];
        rand::fill(&mut b[..]);
        if u16::from(b[0]) < limit {
            out.push(ALPHABET[(u16::from(b[0]) % n) as usize] as char);
        }
    }
    out
}

/// "ABCDEFGH" → "ABCD-EFGH".
pub fn show_code(code: &str) -> String {
    if code.len() == 8 { format!("{}-{}", &code[..4], &code[4..]) } else { code.to_string() }
}

/// A device's name as given: no control characters, at most 60 characters.
fn clean_name(s: &str) -> String {
    let n: String = s.trim().chars().filter(|c| !c.is_control()).take(60).collect();
    if n.is_empty() { "A desktop app".into() } else { n }
}

impl Waiting {
    fn tidy(list: &mut Vec<Request>, t: f64) {
        // Expired ones are kept a little longer, so the app hears "expired" rather than "unknown".
        list.retain(|r| t - r.made < CODE_SECS + 120.0);
    }

    /// A new request: (device_code, user_code), or why not.
    pub fn start(&self, name: &str, from: &str, t: f64) -> Result<(String, String), &'static str> {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        Self::tidy(&mut list, t);
        if list.iter().filter(|r| r.from == from && r.state == State::Waiting).count() >= MAX_WAITING_PER_IP {
            return Err("Too many sign-ins are waiting. Try again in a few minutes.");
        }
        if list.len() >= MAX_WAITING {
            // The list is kept oldest first.
            list.remove(0);
        }
        let device_code = random_hex(32);
        let user_code = loop {
            let c = new_user_code();
            if !list.iter().any(|r| r.user_code == c) {
                break c;
            }
        };
        list.push(Request {
            device_code: device_code.clone(),
            user_code: user_code.clone(),
            name: clean_name(name),
            from: from.to_string(),
            made: t,
            state: State::Waiting,
        });
        Ok((device_code, user_code))
    }

    /// A request still waiting, for the connect page: the device's name, the address it came
    /// from (the same form as `start`'s `from`) and when it was made.
    pub fn about(&self, user_code: &str, t: f64) -> Option<(String, String, f64)> {
        let code = clean_code(user_code);
        let list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        list.iter()
            .find(|r| crate::same(r.user_code.as_bytes(), code.as_bytes()) && r.state == State::Waiting && t - r.made < CODE_SECS)
            .map(|r| (r.name.clone(), r.from.clone(), r.made))
    }

    /// Allows the request with this user code (`mint` makes its device token from the device's
    /// name), or denies it (`mint` None). Only once, and only while it is waiting. The device's
    /// name.
    pub fn answer(&self, user_code: &str, t: f64, mint: Option<Mint>) -> Result<String, &'static str> {
        let code = clean_code(user_code);
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let r = list
            .iter_mut()
            .find(|r| crate::same(r.user_code.as_bytes(), code.as_bytes()))
            .filter(|r| r.state == State::Waiting && t - r.made < CODE_SECS)
            .ok_or("That code isn't right, or it has run out. Start again in the app.")?;
        r.state = match mint {
            Some(mint) => State::Allowed(mint(&r.name).ok_or("The sign-in couldn't be saved. Try again.")?),
            None => State::Denied,
        };
        Ok(r.name.clone())
    }

    /// The app asking. An allowed or denied request is answered once, then forgotten.
    pub fn poll(&self, device_code: &str, t: f64) -> Poll {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let Some(i) = list.iter().position(|r| crate::same(r.device_code.as_bytes(), device_code.as_bytes())) else {
            return Poll::Unknown;
        };
        let r = &list[i];
        if t - r.made >= CODE_SECS && !matches!(r.state, State::Allowed(_)) {
            list.remove(i);
            return Poll::Expired;
        }
        match r.state.clone() {
            State::Waiting => Poll::Waiting,
            State::Allowed(token) => {
                list.remove(i);
                Poll::Allowed(token)
            }
            State::Denied => {
                list.remove(i);
                Poll::Denied
            }
        }
    }
}

// ---------------------------------------------------------------- device tokens

/// One lock for every change to devices.json (sign-ins touch "last used" outside the store lock).
static FILE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Raised whenever a device is removed: open live connections made with a device token check
/// theirs again (live.rs).
pub static REVOKED: LazyLock<tokio::sync::watch::Sender<u64>> = LazyLock::new(|| tokio::sync::watch::channel(0).0);

fn path(store: &Store) -> std::path::PathBuf {
    store.meta().join("devices.json")
}

/// The stored form of a token (never the token itself).
pub fn hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn load(store: &Store) -> Map<String, Value> {
    store.load_json(&path(store)).get("devices").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn save(store: &Store, devices: Map<String, Value>) -> std::io::Result<()> {
    store.save_json(&path(store), &json!({ "devices": devices }))
}

/// How long a device may go unused before it is removed (seconds; None: never).
fn idle_limit() -> Option<f64> {
    let days = crate::setting("STARBASE_DEVICE_IDLE_DAYS").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(IDLE_DAYS);
    (days > 0.0).then_some(days * 86_400.0)
}

/// Whether a stored device has gone unused too long.
fn idle(d: &Value, t: f64) -> bool {
    idle_limit().is_some_and(|limit| t - d.get("used").and_then(Value::as_f64).unwrap_or(0.0) > limit)
}

/// A new device token for `username`: the token (shown to the app once). `sso`: allowed from a
/// session made through single sign-on, so it goes when those do (auth::end_sso_sessions: the
/// sign-in was unlinked, or the person left the allowed group).
pub fn new_token(store: &Store, username: &str, name: &str, sso: bool) -> std::io::Result<String> {
    let _g = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let token = format!("{PREFIX}{}", random_hex(32));
    let mut devices = load(store);
    let t = now();
    // Devices unused too long go now too.
    devices.retain(|_, d| !idle(d, t));
    devices.insert(hash(&token), json!({ "id": random_hex(8), "user": username, "name": clean_name(name), "made": t, "used": t, "sso": sso }));
    save(store, devices)?;
    Ok(token)
}

/// Whose a device token is (and notes it was used). None: not a device token, or removed.
pub fn user_of(store: &Store, token: &str) -> Option<String> {
    if !token.starts_with(PREFIX) {
        return None;
    }
    let h = hash(token);
    let devices = load(store);
    let d = devices.get(&h)?;
    let user = d.get("user").and_then(Value::as_str)?.to_string();
    let t = now();
    if idle(d, t) {
        let _ = remove_where(store, |k, _| k != h);
        return None;
    }
    if t - d.get("used").and_then(Value::as_f64).unwrap_or(0.0) > TOUCH_EVERY {
        let _g = FILE.lock().unwrap_or_else(|e| e.into_inner());
        let mut devices = load(store);
        if let Some(d) = devices.get_mut(&h) {
            d["used"] = json!(t);
            let _ = save(store, devices);
        }
    }
    Some(user)
}

/// Whether the device with this stored hash is still there.
pub fn is_live(store: &Store, token_hash: &str) -> bool {
    load(store).contains_key(token_hash)
}

/// `username`'s devices, for "Your devices" and People: id, name, made, last used, and whether it
/// is the one asking (`current`: a token's hash).
pub fn list(store: &Store, username: &str, current: Option<&str>) -> Vec<Value> {
    let mut out: Vec<Value> = load(store)
        .iter()
        .filter(|(_, d)| d.get("user").and_then(Value::as_str) == Some(username) && !idle(d, now()))
        .map(|(h, d)| json!({ "id": d["id"], "name": d["name"], "made": d["made"], "used": d["used"], "current": current == Some(h.as_str()) }))
        .collect();
    out.sort_by(|a, b| b["used"].as_f64().unwrap_or(0.0).total_cmp(&a["used"].as_f64().unwrap_or(0.0)));
    out
}

fn remove_where(store: &Store, keep: impl Fn(&str, &Value) -> bool) -> std::io::Result<usize> {
    let _g = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let mut devices = load(store);
    let before = devices.len();
    devices.retain(|h, d| keep(h, d));
    let gone = before - devices.len();
    if gone > 0 {
        save(store, devices)?;
        REVOKED.send_modify(|n| *n += 1);
    }
    Ok(gone)
}

/// Removes `username`'s device `id`. False: they have no such device.
pub fn remove(store: &Store, username: &str, id: &str) -> std::io::Result<bool> {
    remove_where(store, |_, d| !(d.get("user").and_then(Value::as_str) == Some(username) && d.get("id").and_then(Value::as_str) == Some(id)))
        .map(|n| n > 0)
}

/// Removes the device whose token this is (the app disconnecting).
pub fn remove_token(store: &Store, token: &str) -> std::io::Result<bool> {
    let h = hash(token);
    remove_where(store, |k, _| k != h).map(|n| n > 0)
}

/// Removes `username`'s devices allowed from a single sign-on session.
pub fn remove_sso(store: &Store, username: &str) {
    let _ =
        remove_where(store, |_, d| d.get("user").and_then(Value::as_str) != Some(username) || d.get("sso").and_then(Value::as_bool) != Some(true));
}

/// Removes every device of `username` (a new password, the account removed).
pub fn remove_all(store: &Store, username: &str) {
    let _ = remove_where(store, |_, d| d.get("user").and_then(Value::as_str) != Some(username));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("starsheets-devices-{}", random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store { root: dir, keep: 2, spaces: false };
        std::fs::create_dir_all(s.meta()).unwrap();
        s
    }

    #[test]
    fn user_codes_are_short_and_plain() {
        for _ in 0..200 {
            let c = new_user_code();
            assert_eq!(c.len(), 8);
            assert!(c.bytes().all(|b| ALPHABET.contains(&b)), "{c}");
        }
        for bad in b"AEIOU0O1IL5S2Z8BUV" {
            assert!(!ALPHABET.contains(bad), "{}", *bad as char);
        }
        assert_eq!(show_code("CDFGHJKM"), "CDFG-HJKM");
        assert_eq!(clean_code(" cdfg-hjkm "), "CDFGHJKM");
    }

    #[test]
    fn a_code_is_used_once_and_runs_out() {
        let w = Waiting::default();
        let mint = |name: &str| Some(format!("token for {name}"));
        let (dc, uc) = w.start("Laptop", "1.2.3.4", 1000.0).unwrap();
        assert_eq!(w.poll(&dc, 1001.0), Poll::Waiting);
        assert_eq!(w.about(&show_code(&uc).to_lowercase(), 1001.0), Some(("Laptop".into(), "1.2.3.4".into(), 1000.0)));
        assert_eq!(w.answer(&uc, 1002.0, Some(&mint)).unwrap(), "Laptop");
        assert!(w.answer(&uc, 1003.0, Some(&mint)).is_err(), "only once");
        assert_eq!(w.poll(&dc, 1004.0), Poll::Allowed("token for Laptop".into()));
        assert_eq!(w.poll(&dc, 1005.0), Poll::Unknown, "the token is handed out once");
        // Running out: no approving after ten minutes, and the app hears so.
        let (dc, uc) = w.start("Old", "1.2.3.4", 1000.0).unwrap();
        assert!(w.about(&uc, 1000.0 + CODE_SECS).is_none());
        assert!(w.answer(&uc, 1000.0 + CODE_SECS, Some(&mint)).is_err());
        assert_eq!(w.poll(&dc, 1000.0 + CODE_SECS), Poll::Expired);
        // Denied.
        let (dc, uc) = w.start("No", "1.2.3.4", 1000.0).unwrap();
        w.answer(&uc, 1001.0, None).unwrap();
        assert_eq!(w.poll(&dc, 1002.0), Poll::Denied);
        assert_eq!(w.poll("nonsense", 1002.0), Poll::Unknown);
        // At most a few waiting from one address.
        for _ in 0..MAX_WAITING_PER_IP {
            w.start("x", "5.5.5.5", 2000.0).unwrap();
        }
        assert!(w.start("x", "5.5.5.5", 2000.0).is_err());
        assert!(w.start("x", "6.6.6.6", 2000.0).is_ok());
        // Full: the oldest goes, new sign-ins still start.
        let w = Waiting::default();
        let (first, _) = w.start("first", "1.0.0.0", 3000.0).unwrap();
        for i in 1..MAX_WAITING {
            w.start("x", &format!("addr{}", i / MAX_WAITING_PER_IP), 3000.0).unwrap();
        }
        assert!(w.start("late", "9.9.9.9", 3000.0).is_ok());
        assert_eq!(w.poll(&first, 3001.0), Poll::Unknown, "the oldest made room");
    }

    #[test]
    fn tokens_are_kept_hashed_and_can_be_removed() {
        let s = store();
        let t = new_token(&s, "ann", "Laptop\u{7}", false).unwrap();
        assert!(t.starts_with(PREFIX) && t.len() > 60);
        let text = std::fs::read_to_string(path(&s)).unwrap();
        assert!(!text.contains(&t) && !text.contains(&t[PREFIX.len()..]), "only the hash is kept");
        assert_eq!(user_of(&s, &t).as_deref(), Some("ann"));
        assert!(user_of(&s, "sbd_wrong").is_none() && user_of(&s, "plain").is_none());
        let l = list(&s, "ann", Some(&hash(&t)));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0]["name"], "Laptop");
        assert_eq!(l[0]["current"], true);
        assert!(list(&s, "bob", None).is_empty());
        let id = l[0]["id"].as_str().unwrap().to_string();
        assert!(!remove(&s, "bob", &id).unwrap(), "only the account's own");
        assert!(remove(&s, "ann", &id).unwrap());
        assert!(user_of(&s, &t).is_none());
        let t2 = new_token(&s, "ann", "Desk", false).unwrap();
        remove_all(&s, "ann");
        assert!(user_of(&s, &t2).is_none());
        // Single sign-on: only the devices allowed from such a session go with it.
        let (plain, sso) = (new_token(&s, "ann", "A", false).unwrap(), new_token(&s, "ann", "B", true).unwrap());
        remove_sso(&s, "ann");
        assert!(user_of(&s, &plain).is_some() && user_of(&s, &sso).is_none());
        // Unused too long (90 days by default): removed.
        let old = new_token(&s, "ann", "Old", false).unwrap();
        {
            let mut d = load(&s);
            d.get_mut(&hash(&old)).unwrap()["used"] = json!(now() - 91.0 * 86_400.0);
            save(&s, d).unwrap();
        }
        assert!(list(&s, "ann", None).iter().all(|d| d["name"] != "Old"), "not listed");
        assert!(user_of(&s, &old).is_none(), "and no longer works");
        assert!(!is_live(&s, &hash(&old)), "removed");
        let _ = std::fs::remove_dir_all(&s.root);
    }
}
