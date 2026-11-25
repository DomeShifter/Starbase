//! "Connect to a server…" for the desktop apps (docs/research/desktop-plan.md, section 4).
//!
//! Signing in works like `gh auth login`: the app asks the server for a code
//! (`POST /api/devices/start`), opens the server's connect page in the browser, where the person
//! signs in as they always do and allows the app, and meanwhile asks the server every few seconds
//! (`POST /api/devices/poll`) until it hands over a device token ([`connect`]). The app never sees
//! a password.
//!
//! The token goes in the system keyring (Secret Service on Linux, Credential Manager, Keychain).
//! Where there is none, it goes in a file only this user can read (`token`, 0600 on Linux and
//! macOS, in [`config_dir`]). Which server it is goes beside it (`server.json`). Every request
//! then carries it as `Authorization: Bearer …` ([`Server::call`]); the server checks it like a
//! browser session, with the same rights. Disconnecting removes it on the server and here.
//!
//! The token is never logged or shown: [`Server`]'s Debug leaves it out.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The keyring's name for Starbase's entries (one per server address).
const KEYRING: &str = "Starbase";

/// A Starbase server this computer is connected to.
#[derive(Clone)]
pub struct Server {
    /// Its address, without a trailing slash (`https://files.example.org`).
    pub url: String,
    /// The account, as the server described it (`username`, `name`, `colour`, `owner`).
    pub user: Value,
    token: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("url", &self.url).field("user", &self.user).finish_non_exhaustive()
    }
}

/// One answer from the server.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub status: u16,
    /// The file's version (ETag), for saving over exactly that version (`If-Match`).
    pub etag: Option<String>,
    /// A file's name and folder on the server (`GET api/files/<id>`: X-File-Name, X-Folder).
    pub name: Option<String>,
    pub folder: Option<String>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// What went wrong, in the server's words when it said.
    pub fn error(&self) -> String {
        match self.json().get("error").and_then(Value::as_str) {
            Some(e) => e.to_string(),
            None => format!("The server answered {}.", self.status),
        }
    }
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent(concat!("Starbase-desktop/", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

/// The biggest answer read (the server's own limit on a file).
const MAX_BODY: u64 = 200 * 1024 * 1024;

fn send(agent: &ureq::Agent, method: &str, url: &str, token: Option<&str>, body: Option<&[u8]>, if_match: Option<&str>) -> Result<Reply, String> {
    let mut req = ureq::http::Request::builder().method(method).uri(url);
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    if let Some(m) = if_match {
        req = req.header("If-Match", m);
    }
    if body.is_some() {
        req = req.header("Content-Type", "application/octet-stream");
    }
    let req = req.body(body.unwrap_or_default().to_vec()).map_err(|e| e.to_string())?;
    let resp = agent.run(req).map_err(|e| format!("The server can't be reached ({e})."))?;
    let status = resp.status().as_u16();
    let header = |n: &str| resp.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    let etag = header("etag");
    // Sent as JSON strings (ASCII only).
    let name = header("x-file-name").and_then(|v| serde_json::from_str::<String>(&v).ok());
    let folder = header("x-folder").and_then(|v| serde_json::from_str::<String>(&v).ok());
    let body = resp.into_body().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())?;
    Ok(Reply { status, etag, name, folder, body })
}

impl Server {
    /// The account's username.
    pub fn username(&self) -> String {
        self.user.get("username").and_then(Value::as_str).unwrap_or("").to_string()
    }

    /// The account's name, for the account menu.
    pub fn name(&self) -> String {
        self.user.get("name").and_then(Value::as_str).unwrap_or("").to_string()
    }

    /// One call to the server's API, `path` relative to the server (`api/files?…`): the same
    /// calls the web app makes. Blocks: call it off the UI thread.
    pub fn call(&self, method: &str, path: &str, body: Option<&[u8]>, if_match: Option<&str>) -> Result<Reply, String> {
        let url = format!("{}/{}", self.url, path.trim_start_matches('/'));
        send(&self.agent, method, &url, Some(&self.token), body, if_match)
    }

    /// Disconnects: the token is removed on the server (when it can be reached) and here.
    pub fn disconnect(&self) -> Result<(), String> {
        let gone = self.call("DELETE", "api/devices?id=this", None, None);
        forget();
        match gone {
            Ok(r) if r.ok() || r.status == 401 || r.status == 404 => Ok(()),
            Ok(r) => Err(r.error()),
            Err(e) => Err(format!("Disconnected here, but the server couldn't be told: {e} Remove this computer from Your devices there.")),
        }
    }
}

// ------------------------------------------------------------ addresses and names

/// The server's address as typed, made whole: `https://` added when no scheme was given,
/// without a trailing slash, path or query.
pub fn clean_url(typed: &str) -> Result<String, String> {
    let t = typed.trim();
    if t.is_empty() {
        return Err("Type the server's address.".into());
    }
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let (scheme, rest) = with_scheme.split_once("://").unwrap_or(("https", &with_scheme));
    let scheme = scheme.to_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err("The address starts with https:// (or http:// on a home network).".into());
    }
    let rest = rest.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.contains(char::is_whitespace) || host.contains('@') {
        return Err("That isn't a server address.".into());
    }
    if scheme == "http" && !on_home_network(host) {
        return Err("Use https:// for this server: over http:// your sign-in would cross the internet unprotected. http:// works only for a server on your own network (an address like 192.168.x.x or 10.x.x.x, or a name ending in .local).".into());
    }
    Ok(format!("{scheme}://{rest}"))
}

/// Whether `host` (with its port, if any) is this computer or on a private network: the only
/// places a plain http:// address is accepted (the token would otherwise travel in the clear).
fn on_home_network(host: &str) -> bool {
    let h = host.to_lowercase();
    let name = match h.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => h.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(h.as_str(), |(n, _)| n),
    };
    if name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") {
        return true;
    }
    match name.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(a)) => a.is_loopback() || a.is_private() || a.is_link_local(),
        Ok(std::net::IpAddr::V6(a)) => {
            let first = a.segments()[0];
            a.is_loopback()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || a.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback() || v4.is_private())
        }
        Err(_) => false,
    }
}

/// What this computer is called on the server's "Your devices" list: "Starbase Sheets on
/// <computer name>".
pub fn device_name(app: &str) -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty());
    match host {
        Some(h) => format!("{app} on {h}"),
        None => app.to_string(),
    }
}

/// Opens `url` in the default browser.
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("The browser couldn't be opened ({e}). Open this address yourself: {url}"))
}

// ------------------------------------------------------------ signing in

/// How signing in is going (from [`connect`]).
#[derive(Debug)]
pub enum Step {
    /// Show this code; the connect page is open in the browser (or `opened` says why not).
    Code {
        user_code: String,
        verify_url: String,
        opened: Result<(), String>,
    },
    /// Still waiting for the person to allow it (sent every few seconds; nothing to show).
    Waiting,
    /// Connected (and the token kept).
    Done(Server),
    Failed(String),
}

/// Signs in to the server at `url` as the app `app` ("Starbase Sheets"), on a thread of its own:
/// read the steps from the answer. Dropping the receiver stops it.
pub fn connect(url: &str, app: &str) -> Receiver<Step> {
    let (tx, rx) = channel();
    let (url, name) = (url.to_string(), device_name(app));
    std::thread::spawn(move || {
        let r = sign_in(&url, &name, &|step| tx.send(step).is_ok());
        let _ = tx.send(match r {
            Ok(server) => Step::Done(server),
            Err(why) => Step::Failed(why),
        });
    });
    rx
}

/// The whole sign-in; `tell` passes a step on (false: nobody is listening any more).
fn sign_in(typed: &str, name: &str, tell: &dyn Fn(Step) -> bool) -> Result<Server, String> {
    let url = clean_url(typed)?;
    let agent = agent();
    let info = send(&agent, "GET", &format!("{url}/api/info"), None, None, None)?;
    if !info.ok() || info.json().get("auth").is_none() {
        return Err("That address doesn't answer like a Starbase server.".into());
    }
    if info.json().get("auth") == Some(&Value::Bool(false)) {
        return Err("Sign-in is off on that server, so it can't be connected to.".into());
    }
    let started = send(
        &agent,
        "POST",
        &format!("{url}/api/devices/start"),
        None,
        Some(&serde_json::to_vec(&json!({ "name": name })).unwrap_or_default()),
        None,
    )?;
    if !started.ok() {
        return Err(started.error());
    }
    let s = started.json();
    let field = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (device_code, user_code) = (field("device_code"), field("user_code"));
    // The connect page at the address the person typed (not one the server names).
    // Without the code: the person types it there (a link with the code in it is what someone
    // phishing would send).
    let verify_url = format!("{url}/api/connect");
    let mut every = Duration::from_secs(s.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, 60));
    let until = Instant::now() + Duration::from_secs_f64(s.get("expires_in").and_then(Value::as_f64).unwrap_or(600.0).clamp(10.0, 3600.0));
    let opened = open_browser(&verify_url);
    if !tell(Step::Code { user_code, verify_url, opened }) {
        return Err("Stopped.".into());
    }
    let body = serde_json::to_vec(&json!({ "device_code": device_code })).unwrap_or_default();
    loop {
        std::thread::sleep(every);
        if Instant::now() > until {
            return Err("The code ran out. Try again.".into());
        }
        // Still wanted? (A probe nobody reads: the window was closed or Cancel pressed.)
        if !tell(Step::Waiting) {
            return Err("Stopped.".into());
        }
        let r = match send(&agent, "POST", &format!("{url}/api/devices/poll"), None, Some(&body), None) {
            Ok(r) => r,
            // A moment without the network: try again next time.
            Err(_) => continue,
        };
        let v = r.json();
        match v.get("status").and_then(Value::as_str).unwrap_or("") {
            "pending" => {}
            "slow_down" => every += Duration::from_secs(5),
            "ok" => {
                let token = v.get("token").and_then(Value::as_str).unwrap_or("").to_string();
                let user = v.get("user").cloned().unwrap_or(Value::Null);
                if token.is_empty() {
                    return Err("The server didn't send a sign-in.".into());
                }
                keep(&url, &token, &user)?;
                return Ok(Server { url, user, token, agent });
            }
            _ => return Err(r.error()),
        }
    }
}

// ------------------------------------------------------------ keeping the token

/// Where Starbase keeps its settings: `$STARBASE_CONFIG_DIR`, else the system's place for them:
/// `$XDG_CONFIG_HOME/starbase` (`~/.config/starbase`) on Linux, `%APPDATA%\Starbase` on Windows,
/// `~/Library/Application Support/Starbase` on macOS.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("STARBASE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = || std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        std::env::var_os("APPDATA").filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("Starbase"))
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support").join("Starbase"))
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))?;
        Some(base.join("starbase"))
    }
}

/// Whether the keyring may be used (STARBASE_NO_KEYRING=1 keeps the token in the file).
fn keyring_allowed() -> bool {
    std::env::var("STARBASE_NO_KEYRING").map_or(true, |v| v.trim().is_empty() || v.trim() == "0")
}

/// Writes `bytes` to `path` readable by this user only (0600), atomically.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Keeps a new connection: the token in the keyring (or the private file), the server beside it.
fn keep(url: &str, token: &str, user: &Value) -> Result<(), String> {
    let dir = config_dir().ok_or("There's no settings folder to keep the connection in.")?;
    let in_keyring = keyring_allowed() && keyring::Entry::new(KEYRING, url).and_then(|e| e.set_password(token)).is_ok();
    if in_keyring {
        let _ = std::fs::remove_file(dir.join("token"));
    } else {
        write_private(&dir.join("token"), token.as_bytes()).map_err(|e| format!("The sign-in couldn't be kept: {e}"))?;
    }
    let saved = json!({ "url": url, "user": user, "keyring": in_keyring });
    write_private(&dir.join("server.json"), &serde_json::to_vec_pretty(&saved).unwrap_or_default())
        .map_err(|e| format!("The connection couldn't be kept: {e}"))
}

/// The server this computer is connected to, if any.
pub fn saved() -> Option<Server> {
    let dir = config_dir()?;
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join("server.json")).ok()?).ok()?;
    let url = v.get("url").and_then(Value::as_str)?.to_string();
    let token = if v.get("keyring") == Some(&Value::Bool(true)) {
        keyring::Entry::new(KEYRING, &url).and_then(|e| e.get_password()).ok()?
    } else {
        std::fs::read_to_string(dir.join("token")).ok()?.trim().to_string()
    };
    (!token.is_empty()).then(|| Server { url, user: v.get("user").cloned().unwrap_or(Value::Null), token, agent: agent() })
}

/// Forgets the connection here (the keyring entry, the files).
pub fn forget() {
    CHANGED.store(true, Ordering::SeqCst);
    let Some(dir) = config_dir() else { return };
    if let Ok(v) = serde_json::from_slice::<Value>(&std::fs::read(dir.join("server.json")).unwrap_or_default())
        && let Some(url) = v.get("url").and_then(Value::as_str)
        && v.get("keyring") == Some(&Value::Bool(true))
    {
        let _ = keyring::Entry::new(KEYRING, url).and_then(|e| e.delete_credential());
    }
    let _ = std::fs::remove_file(dir.join("token"));
    let _ = std::fs::remove_file(dir.join("server.json"));
}

/// Set when this program forgot the connection (disconnected, or the server said the device was
/// removed): the windows look again at what they are connected to ([`take_changed`]).
static CHANGED: AtomicBool = AtomicBool::new(false);

/// What `server.json` looked like when last asked (present, modified time): another Starbase
/// program (Drive, a Docs window) that connects or disconnects changes it, and this one's
/// [`CHANGED`] flag never hears of that.
static SEEN: Mutex<Option<Option<std::time::SystemTime>>> = Mutex::new(None);

fn saved_stamp() -> Option<std::time::SystemTime> {
    let meta = std::fs::metadata(config_dir()?.join("server.json")).ok()?;
    Some(meta.modified().unwrap_or(std::time::UNIX_EPOCH))
}

/// Whether the connection changed since the last time this was asked: forgotten here, or
/// connected/disconnected by another Starbase program on this computer.
pub fn take_changed() -> bool {
    let here = CHANGED.swap(false, Ordering::SeqCst);
    let now = saved_stamp();
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let elsewhere = seen.is_some_and(|before| before != now);
    *seen = Some(now);
    here || elsewhere
}

impl Server {
    /// The sign-in kept now for the same server, when it is a newer one than this (the person
    /// connected again while this window was open).
    pub fn renewed(&self) -> Option<Server> {
        saved().filter(|s| s.url == self.url && s.token != self.token)
    }

    /// The server said this token no longer works (the device was removed there): forget it here
    /// too, unless a newer sign-in has replaced it meanwhile.
    pub fn forget_if_current(&self) {
        if saved().is_some_and(|s| s.url == self.url && s.token == self.token) {
            forget();
        }
    }
}

/// A connection for something that stays open (a document's window): when the server refuses its
/// token, it takes up a newer sign-in made meanwhile (connected again) and tries once more; with
/// none, it forgets the removed one, so the menus offer "Connect to a server…" again.
#[derive(Clone)]
pub struct Renewing(Arc<Mutex<Server>>);

impl Renewing {
    pub fn new(server: Server) -> Renewing {
        Renewing(Arc::new(Mutex::new(server)))
    }

    /// The connection as it is now.
    pub fn server(&self) -> Server {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// As [`Server::call`]; a 401 that a newer sign-in can't fix comes back as it is.
    pub fn call(&self, method: &str, path: &str, body: Option<&[u8]>, if_match: Option<&str>) -> Result<Reply, String> {
        let server = self.server();
        let r = server.call(method, path, body, if_match)?;
        if r.status != 401 {
            return Ok(r);
        }
        match server.renewed() {
            Some(newer) => {
                *self.0.lock().unwrap_or_else(|e| e.into_inner()) = newer.clone();
                newer.call(method, path, body, if_match)
            }
            None => {
                server.forget_if_current();
                Ok(r)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_made_whole() {
        assert_eq!(clean_url("files.example.org").unwrap(), "https://files.example.org");
        assert_eq!(clean_url(" HTTP://nas.local:8080/ ").unwrap(), "http://nas.local:8080");
        // Plain http:// only on this computer or a private network.
        for ok in [
            "http://192.168.1.5:8080",
            "http://10.0.0.2",
            "http://172.20.1.1",
            "http://127.0.0.1:3000",
            "http://localhost",
            "http://[::1]:8080",
            "http://[fd12::5]",
        ] {
            assert!(clean_url(ok).is_ok(), "{ok}");
        }
        for bad in ["http://files.example.org", "http://8.8.8.8", "http://172.32.0.1", "http://[2001:db8::1]", "http://example.local.evil.com"] {
            assert!(clean_url(bad).unwrap_err().contains("https://"), "{bad}");
        }
        assert!(clean_url("https://8.8.8.8").is_ok());
        assert_eq!(clean_url("https://example.org/sheets/?x=1").unwrap(), "https://example.org/sheets");
        assert!(clean_url("").is_err());
        assert!(clean_url("ftp://x").is_err());
        assert!(clean_url("https://user@evil").is_err());
        assert!(clean_url("https:///nothing").is_err());
    }

    #[test]
    fn the_token_file_is_private_and_never_in_debug() {
        // One test touches the environment, so the two checks share it.
        let dir = std::env::temp_dir().join(format!("starbase-connect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: only this test in this crate reads or writes these variables.
        unsafe {
            std::env::set_var("STARBASE_CONFIG_DIR", &dir);
            std::env::set_var("STARBASE_NO_KEYRING", "1");
        }
        assert!(saved().is_none());
        keep("https://example.org", "sbd_secret", &json!({ "username": "ann", "name": "Ann" })).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for f in ["token", "server.json"] {
                assert_eq!(std::fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777, 0o600, "{f}");
            }
        }
        assert!(!std::fs::read_to_string(dir.join("server.json")).unwrap().contains("sbd_secret"));
        let s = saved().unwrap();
        assert_eq!((s.url.as_str(), s.username(), s.name()), ("https://example.org", "ann".to_string(), "Ann".to_string()));
        assert!(!format!("{s:?}").contains("sbd_secret"), "never shown");
        // Connected again while a window had the old sign-in: it takes up the new one, and a
        // refusal of the old one doesn't forget the new.
        assert!(s.renewed().is_none());
        keep("https://example.org", "sbd_newer", &json!({ "username": "ann" })).unwrap();
        let newer = s.renewed().unwrap();
        let _ = take_changed();
        s.forget_if_current();
        assert!(saved().is_some() && !take_changed(), "the newer sign-in stays");
        newer.forget_if_current();
        assert!(saved().is_none() && take_changed(), "a removed device is forgotten here too");
        // Another Starbase program connects, then disconnects: this one notices both.
        assert!(!take_changed());
        keep("https://example.org", "sbd_other", &json!({ "username": "ann" })).unwrap();
        assert!(take_changed(), "connected by another program");
        assert!(!take_changed(), "told once");
        std::fs::remove_file(dir.join("server.json")).unwrap();
        assert!(take_changed(), "disconnected by another program");
        keep("https://example.org", "sbd_secret", &json!({ "username": "ann" })).unwrap();
        forget();
        assert!(saved().is_none() && !dir.join("token").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_names_say_which_app() {
        assert!(device_name("Starbase Sheets").starts_with("Starbase Sheets"));
    }
}
