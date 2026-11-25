//! "Connect to a server…" for the desktop apps (docs/research/desktop-plan.md, section 4).
//!
//! Signing in works like `gh auth login`: the app asks the server for a code
//! (`POST /api/devices/start`), opens the server's connect page in the browser, where the person
//! signs in as they always do and allows the app, and meanwhile asks the server every few seconds
//! (`POST /api/devices/poll`) until it hands over a device token ([`connect`]). The app never sees
//! a password.
//!
//! The token goes in the system keyring (Secret Service on Linux, Credential Manager, Keychain).
//! Where there is none, it goes in a file only this user can read (`starbase/token`, 0600, in
//! `$XDG_CONFIG_HOME`). Which server it is goes beside it (`starbase/server.json`). Every request
//! then carries it as `Authorization: Bearer …` ([`Server::call`]); the server checks it like a
//! browser session, with the same rights. Disconnecting removes it on the server and here.
//!
//! The token is never logged or shown: [`Server`]'s Debug leaves it out.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
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
    Ok(format!("{scheme}://{rest}"))
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
    let verify_url = format!("{url}/connect?code={user_code}");
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

/// Where Starbase keeps its settings: `$STARBASE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/starbase`
/// (`~/.config/starbase`; `%APPDATA%\starbase` on Windows).
pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("STARBASE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| {
        if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
        }
    })?;
    Some(base.join("starbase"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_made_whole() {
        assert_eq!(clean_url("files.example.org").unwrap(), "https://files.example.org");
        assert_eq!(clean_url(" HTTP://nas.local:8080/ ").unwrap(), "http://nas.local:8080");
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
        forget();
        assert!(saved().is_none() && !dir.join("token").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_names_say_which_app() {
        assert!(device_name("Starbase Sheets").starts_with("Starbase Sheets"));
    }
}
