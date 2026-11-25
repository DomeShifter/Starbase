//! Document-portal paths back to the real path on the computer (inside the Flatpak).
//!
//! The Flatpak can't see network shares, so the file manager hands it a file through the
//! document portal: `/run/user/<uid>/doc/<id>/Budget.xlsx` (or `/run/flatpak/doc/<id>/…`), and the
//! folder picker answers the same way. Such a path never matches the server's storage folder, so
//! it is turned back into the path on the computer first:
//!
//! 1. the `user.document-portal.host-path` extended attribute the portal puts on its files
//!    (xdg-desktop-portal 1.19+; before 1.19.3 the value ended in a NUL, which is dropped);
//! 2. else the portal's `GetHostPaths` method (Documents interface version 5, allowed inside the
//!    sandbox for documents the app can read), through `gdbus`, for the document's id; a file
//!    inside a shared folder keeps its path below that folder.
//!
//! Only paths in the portal's own folders are looked at: an attribute on any other file is never
//! trusted to say where it "really" is.

use std::path::{Component, Path, PathBuf};

/// The attribute the document portal sets on its files.
pub const HOST_PATH_ATTR: &str = "user.document-portal.host-path";

/// For a path inside the document portal: (its document id, the part below the document's own
/// entry, e.g. `Work/Budget.xlsx` for a file in a shared folder, or `Budget.xlsx`).
pub fn portal_parts(path: &Path) -> Option<(String, PathBuf)> {
    let mut parts: Vec<&str> = Vec::new();
    for c in path.components() {
        match c {
            Component::RootDir => {}
            Component::Normal(s) => parts.push(s.to_str()?),
            // `..`, `.` or a relative path: not one of the portal's own.
            _ => return None,
        }
    }
    if !path.is_absolute() {
        return None;
    }
    let rest = match parts.as_slice() {
        ["run", "user", uid, "doc", rest @ ..] if uid.chars().all(|c| c.is_ascii_digit()) => rest,
        ["run", "flatpak", "doc", rest @ ..] => rest,
        _ => return None,
    };
    // `by-app/<app id>/<doc id>/…` is the same store, seen per app.
    let rest = match rest {
        ["by-app", _, tail @ ..] => tail,
        tail => tail,
    };
    let (id, below) = rest.split_first()?;
    if id.is_empty() || below.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((id.to_string(), below.iter().collect()))
}

/// The real path of `path` when it is a document-portal path and the portal says where it is;
/// otherwise `path` itself.
pub fn host_path(path: &Path) -> PathBuf {
    let Some((id, below)) = portal_parts(path) else { return path.to_path_buf() };
    if let Some(p) = attr_path(path) {
        return p;
    }
    // The portal's entry is the document itself (a file) or a shared folder: `below` starts with
    // the entry's own name, which GetHostPaths' answer already ends in.
    if let Some(entry) = get_host_path(&id) {
        let mut tail = below.components();
        tail.next();
        return entry.join(tail.as_path());
    }
    path.to_path_buf()
}

/// The host path the portal wrote in the file's attribute.
pub fn attr_path(path: &Path) -> Option<PathBuf> {
    let v = read_attr(path, HOST_PATH_ATTR)?;
    let v = v.strip_suffix(b"\0").unwrap_or(&v);
    use std::os::unix::ffi::OsStrExt as _;
    let p = PathBuf::from(std::ffi::OsStr::from_bytes(v));
    p.is_absolute().then_some(p)
}

/// An extended attribute's value (follows links, as opening the file does).
pub fn read_attr(path: &Path, name: &str) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt as _;
    let p = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let n = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0u8; 4096];
    // SAFETY: both names are NUL-terminated C strings and `buf` is writable for its whole length.
    let len = unsafe { libc::getxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if len <= 0 {
        return None;
    }
    buf.truncate(len as usize);
    Some(buf)
}

/// The portal's `GetHostPaths` for one document id, through `gdbus` (in the Flatpak runtime).
fn get_host_path(id: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Documents",
            "--object-path",
            "/org/freedesktop/portal/documents",
            "--method",
            "org.freedesktop.portal.Documents.GetHostPaths",
            &format!("['{id}']"),
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_gdbus_bytes(&String::from_utf8_lossy(&out.stdout))
}

/// The path in gdbus's printout of GetHostPaths' answer: `({'<id>': b'/home/ann/x'},)`. A byte
/// string is printed as `b'…'` with backslash escapes (`\\`, `\'`, octal `\NNN`), or, when it
/// isn't text, as a list of numbers (`[byte 0x2f, 0x68, …]`).
pub fn parse_gdbus_bytes(s: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    let after = &s[s.find(": ")? + 2..];
    let mut out = Vec::new();
    if let Some(body) = after.strip_prefix("b'").or_else(|| after.strip_prefix("b\"")) {
        let quote = after.as_bytes()[1];
        let b = body.as_bytes();
        let mut i = 0;
        loop {
            let c = *b.get(i)?;
            if c == quote {
                break;
            }
            if c == b'\\' {
                let e = *b.get(i + 1)?;
                if e.is_ascii_digit() {
                    let digits: Vec<u8> = b[i + 1..].iter().take(3).take_while(|d| (b'0'..=b'7').contains(d)).copied().collect();
                    out.push(u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 8).ok()?);
                    i += 1 + digits.len();
                    continue;
                }
                out.push(match e {
                    b'n' => b'\n',
                    b't' => b'\t',
                    other => other,
                });
                i += 2;
                continue;
            }
            out.push(c);
            i += 1;
        }
    } else {
        let list = after.strip_prefix('[')?;
        for n in list.split(']').next()?.split(',') {
            let n = n.trim().trim_start_matches("byte").trim();
            let n = n.strip_prefix("0x").map_or_else(|| n.parse().ok(), |h| u8::from_str_radix(h, 16).ok())?;
            out.push(n);
        }
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    let p = PathBuf::from(std::ffi::OsString::from_vec(out));
    p.is_absolute().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_paths_are_recognised() {
        let p = |s: &str| portal_parts(Path::new(s));
        assert_eq!(p("/run/user/1000/doc/a1b2c3d4/Budget.xlsx"), Some(("a1b2c3d4".into(), "Budget.xlsx".into())));
        assert_eq!(p("/run/user/1000/doc/a1b2/Share/Work/Plan.ods"), Some(("a1b2".into(), "Share/Work/Plan.ods".into())));
        assert_eq!(p("/run/flatpak/doc/ff00/Letter.docx"), Some(("ff00".into(), "Letter.docx".into())));
        assert_eq!(p("/run/user/1000/doc/by-app/io.github.DomeShifter.Starbase/a1/B.xlsx"), Some(("a1".into(), "B.xlsx".into())));
        for not in [
            "/home/ann/Budget.xlsx",
            "/run/user/1000/doc/a1b2",
            "/run/user/x/doc/a1/B.xlsx",
            "/run/user/1000/doc/../etc/B.xlsx",
            "/tmp/run/user/1/doc/a/B",
        ] {
            assert_eq!(p(not), None, "{not}");
        }
        // Not a portal path: left alone (an attribute on it is never read).
        assert_eq!(host_path(Path::new("/home/ann/Budget.xlsx")), PathBuf::from("/home/ann/Budget.xlsx"));
    }

    #[test]
    fn gdbus_answers_are_read() {
        let p = |s: &str| parse_gdbus_bytes(s);
        assert_eq!(p("({'a1': b'/mnt/nas/starbase/ann/Budget.xlsx'},)\n"), Some("/mnt/nas/starbase/ann/Budget.xlsx".into()));
        assert_eq!(p("({'a1': b'/mnt/nas/Ann\\'s \\303\\251.xlsx\\000'},)"), Some("/mnt/nas/Ann's é.xlsx".into()));
        assert_eq!(p("({'a1': b\"/x/it's\"},)"), Some("/x/it's".into()));
        assert_eq!(p("({'a1': [byte 0x2f, 0x78, 0xff]},)").map(|p| p.into_os_string().into_encoded_bytes()), Some(b"/x\xff".to_vec()));
        for bad in ["({},)", "({'a1': b'relative'},)", "Error: GDBus.Error", ""] {
            assert_eq!(p(bad), None, "{bad}");
        }
    }

    /// A fake portal file: the attribute set on a temp file is read back (the portal's own
    /// folders can't be made in a test, so this checks the attribute half on its own).
    #[test]
    fn the_portal_attribute_is_read() {
        use std::os::unix::ffi::OsStrExt as _;
        let dir = std::env::temp_dir().join(format!("starbase-portal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Budget.xlsx");
        std::fs::write(&file, "x").unwrap();
        let set = |value: &[u8]| {
            let p = std::ffi::CString::new(file.as_os_str().as_bytes()).unwrap();
            let n = std::ffi::CString::new(HOST_PATH_ATTR).unwrap();
            // SAFETY: NUL-terminated names; `value` is readable for its length.
            unsafe { libc::setxattr(p.as_ptr(), n.as_ptr(), value.as_ptr().cast(), value.len(), 0) == 0 }
        };
        if !set(b"/mnt/nas/starbase/ann/Budget.xlsx") {
            eprintln!("skipped: this file system doesn't keep user attributes");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert_eq!(attr_path(&file), Some("/mnt/nas/starbase/ann/Budget.xlsx".into()));
        assert!(set(b"/mnt/nas/starbase/ann/Budget.xlsx\0"), "before 1.19.3 the value ended in a NUL");
        assert_eq!(attr_path(&file), Some("/mnt/nas/starbase/ann/Budget.xlsx".into()));
        assert!(set(b"relative/path"));
        assert_eq!(attr_path(&file), None);
        assert_eq!(attr_path(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
