//! The decisions `starbase-open` makes, without side effects (so they can be tested): where the
//! server's storage folder is on this computer, whether a file is inside it (and its path there),
//! the web address a found file opens at, and which desktop program opens anything else.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// The setting's file, beside the connection's `server.json` in Starbase's settings folder.
pub const SETTING: &str = "open.json";

/// "This folder on my computer is the server's storage", for one server.
#[derive(Clone, Debug, PartialEq)]
pub struct Storage {
    /// The server it was answered for (the setting is asked again for another one).
    pub server: String,
    /// The folder; `None` = asked, and the storage isn't on this computer (don't ask again).
    pub folder: Option<PathBuf>,
}

/// The setting for `server`, if it was answered for that server.
pub fn load(dir: &Path, server: &str) -> Option<Storage> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join(SETTING)).ok()?).ok()?;
    let saved = v.get("server").and_then(Value::as_str)?;
    if saved != server {
        return None;
    }
    let folder = v.get("folder").and_then(Value::as_str).filter(|f| !f.is_empty()).map(PathBuf::from);
    Some(Storage { server: saved.to_string(), folder })
}

pub fn save(dir: &Path, s: &Storage) -> std::io::Result<()> {
    let folder = s.folder.as_ref().map(|f| f.to_string_lossy().into_owned());
    // Only this user can read it, like server.json beside it.
    let bytes = serde_json::to_vec_pretty(&json!({ "server": s.server, "folder": folder })).unwrap_or_default();
    starbase_connect::write_private(&dir.join(SETTING), &bytes)
}

/// Forgets the setting (asked again next time).
pub fn forget(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(SETTING));
}

/// `path` made absolute (against `cwd`) with `.` and `..` worked out by name, without looking at
/// the disk (a share that is slow or gone mustn't hold this up).
pub fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The path of `file` inside the storage `folder` as the server names it (`ann/Budget.xlsx`), or
/// `None` when the file isn't in there. Whole folder names only (`/mnt/nas2` isn't inside
/// `/mnt/nas`); hidden parts (the bin, `.starsheets`) are never the server's files.
pub fn inside(folder: &Path, file: &Path) -> Option<String> {
    let rest = file.strip_prefix(folder).ok()?;
    let mut parts = Vec::new();
    for c in rest.components() {
        let Component::Normal(s) = c else { return None };
        let s = s.to_str()?;
        if s.starts_with('.') || s.contains('\\') {
            return None;
        }
        parts.push(s);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Text for a URL query value: everything but letters, digits, `-._~` and `/` percent-encoded.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The page a found file opens on (the server's answer to `api/find-path`): the spreadsheet page
/// or Docs' page beside it, as the web app's own links. Built here from the id, never taken as an
/// address from the answer, so the server can only ever send the browser to itself. The saved
/// server address must be http:// or https:// ([`web_address`]): anything else could open a
/// local file (on Windows the browser is started through the system's "open this" handler).
pub fn web_url(server: &str, found: &Value) -> Result<String, String> {
    if !web_address(server) {
        return Err(not_web(server));
    }
    let id = found.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("the server's answer wasn't understood".into());
    }
    let page = if found.get("kind").and_then(Value::as_str) == Some("doc") { "docs/" } else { "" };
    Ok(format!("{}/{page}?file={id}", server.trim_end_matches('/')))
}

/// Whether a saved server address is a web address: http:// or https://, with a host.
pub fn web_address(server: &str) -> bool {
    let lower = server.to_ascii_lowercase();
    lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

/// Why nothing is opened from a saved address that isn't a web address.
pub fn not_web(server: &str) -> String {
    format!("the saved server address ({server}) doesn't start with https:// or http://, so nothing is opened from it; connect to the server again")
}

/// The desktop program for a file: Starbase Slides for presentations, Starbase Docs for
/// documents, else Starbase Sheets (as the AppImage's AppRun decides).
pub fn program_for(file: &Path) -> &'static str {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pptx" | "potx" | "ppsx" => "starslides",
        "docx" | "docm" | "dotx" | "dotm" | "doc" | "odt" | "rtf" => "stardocs",
        _ => "starsheets",
    }
}

/// Whether `folder` looks like a Starbase storage folder (the server keeps `.starsheets` in it).
pub fn looks_like_storage(folder: &Path) -> bool {
    folder.join(".starsheets").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_the_storage_folder() {
        let nas = Path::new("/mnt/nas/starbase");
        assert_eq!(inside(nas, Path::new("/mnt/nas/starbase/ann/Budget.xlsx")).as_deref(), Some("ann/Budget.xlsx"));
        assert_eq!(inside(nas, Path::new("/mnt/nas/starbase/ann/Work 2/Plan (2).ods")).as_deref(), Some("ann/Work 2/Plan (2).ods"));
        assert_eq!(inside(Path::new("/mnt/nas/starbase/"), Path::new("/mnt/nas/starbase/a.csv")).as_deref(), Some("a.csv"));
        for outside in [
            "/mnt/nas/starbase2/ann/Budget.xlsx",
            "/mnt/nas/Budget.xlsx",
            "/home/ann/Budget.xlsx",
            "/mnt/nas/starbase",
            "/mnt/nas/starbase/ann/.bin/Old.xlsx",
            "/mnt/nas/starbase/.starsheets/index.json",
            "relative/ann/Budget.xlsx",
        ] {
            assert_eq!(inside(nas, Path::new(outside)), None, "{outside}");
        }
    }

    #[test]
    fn dots_are_worked_out_before_matching() {
        let cwd = Path::new("/mnt/nas/starbase/ann");
        assert_eq!(absolute(Path::new("Budget.xlsx"), cwd), PathBuf::from("/mnt/nas/starbase/ann/Budget.xlsx"));
        assert_eq!(absolute(Path::new("./Work/../Budget.xlsx"), cwd), PathBuf::from("/mnt/nas/starbase/ann/Budget.xlsx"));
        // Climbing out of the storage folder by name leaves it.
        let out = absolute(Path::new("/mnt/nas/starbase/ann/../../secret.xlsx"), cwd);
        assert_eq!(inside(Path::new("/mnt/nas/starbase"), &out), None);
        assert_eq!(absolute(Path::new("/../../x"), cwd), PathBuf::from("/x"));
    }

    #[test]
    fn the_web_address_is_built_here() {
        let s = "https://files.example.org";
        assert_eq!(web_url(s, &json!({"id": "a1b2c3", "kind": "sheet"})).as_deref(), Ok("https://files.example.org/?file=a1b2c3"));
        assert_eq!(web_url("https://x.org/sb/", &json!({"id": "a1", "kind": "doc"})).as_deref(), Ok("https://x.org/sb/docs/?file=a1"));
        assert!(web_url("HTTP://nas.local:8080", &json!({"id": "a1"})).is_ok());
        for bad in [json!({}), json!({"id": ""}), json!({"id": "x&y=1"}), json!({"id": "//evil.example"}), json!({"id": 5})] {
            assert!(web_url(s, &bad).is_err(), "{bad}");
        }
        // Only web addresses: an edited server.json can't make the browser open a local file.
        for bad in ["file:///etc", "C:\\x", "javascript:alert(1)", "https://", "https:///x", "smb://nas/share", ""] {
            assert!(web_url(bad, &json!({"id": "a1"})).unwrap_err().contains("https://"), "{bad}");
        }
    }

    #[test]
    fn queries_are_encoded() {
        assert_eq!(encode("ann/Work 2/Plan & co.xlsx"), "ann/Work%202/Plan%20%26%20co.xlsx");
        assert_eq!(encode("é#?"), "%C3%A9%23%3F");
    }

    #[test]
    fn documents_open_in_docs() {
        assert_eq!(program_for(Path::new("/a/Letter.DOCX")), "stardocs");
        assert_eq!(program_for(Path::new("/a/Notes.odt")), "stardocs");
        assert_eq!(program_for(Path::new("/a/Talk.PPTX")), "starslides");
        assert_eq!(program_for(Path::new("/a/Budget.xlsx")), "starsheets");
        assert_eq!(program_for(Path::new("/a/data.csv")), "starsheets");
    }

    #[test]
    fn the_setting_is_per_server() {
        let dir = std::env::temp_dir().join(format!("starbase-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir, "https://a.example"), None);
        let s = Storage { server: "https://a.example".into(), folder: Some("/mnt/nas/starbase".into()) };
        save(&dir, &s).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(dir.join(SETTING)).unwrap().permissions().mode() & 0o777, 0o600, "this user only");
        }
        assert_eq!(load(&dir, "https://a.example"), Some(s));
        assert_eq!(load(&dir, "https://b.example"), None, "another server: asked again");
        let no = Storage { server: "https://a.example".into(), folder: None };
        save(&dir, &no).unwrap();
        assert_eq!(load(&dir, "https://a.example"), Some(no), "answered 'not on this computer'");
        assert!(!looks_like_storage(&dir));
        std::fs::create_dir_all(dir.join(".starsheets")).unwrap();
        assert!(looks_like_storage(&dir));
        forget(&dir);
        assert_eq!(load(&dir, "https://a.example"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
