//! `starbase-open`: what a file manager runs when a spreadsheet or document is opened
//! (packaging/linux/io.github.DomeShifter.Starbase.Open.desktop).
//!
//! A file inside the connected Starbase server's storage folder, as seen on this computer (the
//! network share, mounted and browsed in the file manager), opens on the server in the browser:
//! live editing, sharing and versions, and no desktop program writing to the share behind the
//! server's back (which makes conflicted copies). Every other file, or when the server can't
//! find it for this person, opens in Starbase Sheets or Starbase Docs as before.
//!
//! The server comes from "Connect to a server…" (starbase_connect::saved). Where its storage
//! folder is on this computer is asked once per server and kept beside the connection
//! (`open.json`, map.rs); the file's path inside that folder is sent to the server's
//! `api/find-path`, which answers only for files this person can open.
//!
//!   starbase-open FILE…                    open each file
//!   starbase-open --explain FILE           print what would happen (JSON), open nothing
//!   starbase-open --storage-folder DIR     set the folder ("none": not on this computer)
//!   starbase-open --ask-again              forget the answer; asked at the next open

mod map;
#[cfg(target_os = "linux")]
mod portal;
/// The document portal is Linux's (Flatpak): elsewhere a path is what it says.
#[cfg(not(target_os = "linux"))]
mod portal {
    pub fn host_path(p: &std::path::Path) -> std::path::PathBuf {
        p.to_path_buf()
    }
}

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::json;

/// Where a file opens.
enum Plan {
    /// On the server, at this address.
    Web(String),
    /// In a desktop program (`starsheets`/`stardocs`), and why.
    Desktop(&'static str, String),
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let first = args.first().and_then(|a| a.to_str()).unwrap_or("");
    let code = match first {
        "--version" | "-V" => {
            println!("starbase-open {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "--help" | "-h" => {
            println!("{}", HELP.trim());
            0
        }
        "--storage-folder" => set_folder(args.get(1)),
        "--ask-again" => {
            if let Some(dir) = starbase_connect::config_dir() {
                map::forget(&dir);
            }
            0
        }
        "--explain" => {
            let mut asked = Asked::Never;
            for f in &args[1..] {
                let path = absolute(Path::new(f));
                let mut v = match plan(&path, &mut asked) {
                    Plan::Web(url) => json!({ "file": path.display().to_string(), "opens": "web", "url": url }),
                    Plan::Desktop(prog, why) => json!({ "file": path.display().to_string(), "opens": prog, "why": why }),
                };
                // A document-portal path: where it really is (what was matched).
                let host = portal::host_path(&path);
                if host != path {
                    v["host"] = json!(host.display().to_string());
                }
                println!("{v}");
            }
            0
        }
        _ => {
            let mut asked = Asked::Not;
            let mut failed = 0;
            for f in args.iter().filter(|a| !a.to_string_lossy().starts_with("--")) {
                let path = absolute(Path::new(f));
                let opened = match plan(&path, &mut asked) {
                    Plan::Web(url) => starbase_connect::open_browser(&url).or_else(|e| {
                        eprintln!("starbase-open: {e}");
                        launch(map::program_for(&path), &path)
                    }),
                    Plan::Desktop(prog, why) => {
                        eprintln!("starbase-open: {}: {why}", path.display());
                        launch(prog, &path)
                    }
                };
                if let Err(e) = opened {
                    eprintln!("starbase-open: {e}");
                    tell_error(&e);
                    failed += 1;
                }
            }
            i32::from(failed > 0)
        }
    };
    std::process::exit(code);
}

const HELP: &str = "
starbase-open: opens spreadsheets and documents with Starbase.
A file in your Starbase server's storage folder opens on the server, in your browser;
any other file opens in Starbase Sheets or Starbase Docs.

  starbase-open FILE...                 open each file
  starbase-open --explain FILE...       say what would happen (JSON lines), open nothing
  starbase-open --storage-folder DIR    the server's storage folder on this computer
  starbase-open --storage-folder none   the server's storage isn't on this computer
  starbase-open --ask-again             forget the answer: asked at the next open
";

fn absolute(p: &Path) -> PathBuf {
    map::absolute(p, &std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

/// Whether the storage-folder question may be (or was) asked in this run.
#[derive(Clone, Copy, PartialEq)]
enum Asked {
    /// Not asked yet: ask when needed.
    Not,
    /// Asked (or skipped) in this run: not again for the next file.
    Done,
    /// Never ask (--explain).
    Never,
}

/// Where `file` opens.
fn plan(file: &Path, asked: &mut Asked) -> Plan {
    let desktop = |why: &str| Plan::Desktop(map::program_for(file), why.to_string());
    let Some(server) = starbase_connect::saved() else { return desktop("not connected to a server") };
    let Some(dir) = starbase_connect::config_dir() else { return desktop("no settings folder") };
    // Nothing is sent to, or opened from, a saved address that isn't http:// or https://.
    if !map::web_address(&server.url) {
        return desktop(&map::not_web(&server.url));
    }
    let storage = match map::load(&dir, &server.url) {
        Some(s) => s,
        None if *asked == Asked::Not => {
            *asked = Asked::Done;
            match ask_folder(&server.url) {
                Some(s) => {
                    if let Err(e) = map::save(&dir, &s) {
                        eprintln!("starbase-open: the answer couldn't be kept ({e})");
                    }
                    s
                }
                None => return desktop("the server's storage folder hasn't been set"),
            }
        }
        None => return desktop("the server's storage folder hasn't been set"),
    };
    let Some(folder) = storage.folder else { return desktop("the server's storage isn't on this computer") };
    // Inside the Flatpak a file (and a folder picked there) can arrive as a document-portal path:
    // compared by where it really is on the computer (portal.rs).
    let (folder, file) = (portal::host_path(&folder), portal::host_path(file));
    // By name first; then with links worked out (a share reached through a link, or a link to it).
    let rel = map::inside(&folder, &file).or_else(|| {
        let (f, p) = (std::fs::canonicalize(&folder).ok()?, std::fs::canonicalize(&file).ok()?);
        map::inside(&f, &p)
    });
    let Some(rel) = rel else { return desktop("not in the server's storage folder") };
    let reply = match server.call("GET", &format!("api/find-path?path={}", map::encode(&rel)), None, None) {
        Ok(r) => r,
        Err(e) => return desktop(&format!("the server didn't answer ({e})")),
    };
    match reply.status {
        200 => match map::web_url(&server.url, &reply.json()) {
            Ok(url) => Plan::Web(url),
            Err(why) => desktop(&why),
        },
        401 => {
            server.forget_if_current();
            desktop("this computer is no longer connected to the server")
        }
        _ => desktop(&format!("the server says: {}", reply.error())),
    }
}

/// Asks once whether the server's storage folder is on this computer, and which folder it is.
/// `None`: not answered (closed, or no desktop to ask on): asked again next time.
fn ask_folder(server: &str) -> Option<map::Storage> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return None;
    }
    let yes = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title("Starbase")
        .set_description(format!(
            "Is your Starbase server's storage folder on this computer?\n\nYou're connected to {server}. If the folder where it keeps its files is also on this computer (a network share, for example), files you open from that folder open on the server, in your browser: live editing, sharing and versions.\n\nChoose Yes to pick that folder. Choose No to always open files in Starbase Sheets and Docs. (Change this later with: starbase-open --ask-again)"
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    let answer = |folder| Some(map::Storage { server: server.to_string(), folder });
    match yes {
        rfd::MessageDialogResult::Yes => {}
        rfd::MessageDialogResult::No => return answer(None),
        _ => return None,
    }
    let folder = rfd::FileDialog::new().set_title("The Starbase server's storage folder").pick_folder()?;
    if !map::looks_like_storage(&folder) {
        let anyway = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Starbase")
            .set_description(format!(
                "{} doesn't look like a Starbase storage folder: the server keeps a hidden .starsheets folder in it, and there isn't one.\n\nUse it anyway?",
                folder.display()
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if anyway != rfd::MessageDialogResult::Yes {
            return None;
        }
    }
    // The folder picker inside the Flatpak answers with a document-portal path: keep the real one.
    answer(Some(portal::host_path(&folder)))
}

/// `--storage-folder DIR|none`, for the connected server.
fn set_folder(arg: Option<&OsString>) -> i32 {
    let Some(arg) = arg else {
        eprintln!("starbase-open: --storage-folder needs a folder (or none)");
        return 2;
    };
    let (Some(server), Some(dir)) = (starbase_connect::saved(), starbase_connect::config_dir()) else {
        eprintln!("starbase-open: connect to a server first (Connect to a server… in Starbase)");
        return 1;
    };
    let folder = if arg == "none" {
        None
    } else {
        let f = absolute(Path::new(arg));
        if !f.is_dir() {
            eprintln!("starbase-open: {} isn't a folder", f.display());
            return 1;
        }
        if !map::looks_like_storage(&f) {
            eprintln!("starbase-open: note: {} has no .starsheets folder, so it may not be the server's storage", f.display());
        }
        Some(portal::host_path(&f))
    };
    match map::save(&dir, &map::Storage { server: server.url, folder }) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("starbase-open: {e}");
            1
        }
    }
}

/// Starts `name` (`starsheets`/`stardocs`) on `file`, on its own. Inside the AppImage
/// (`$APPIMAGE`) that is the AppImage with the program's name first (its AppRun picks the
/// program); otherwise the program beside this one (the Flatpak, the tar), else on PATH.
fn launch(name: &str, file: &Path) -> Result<(), String> {
    let mut cmd = match std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file()) {
        Some(image) => {
            let mut c = std::process::Command::new(image);
            c.arg(name);
            c
        }
        None => {
            let exe = program(name).ok_or_else(|| format!("{name} isn't installed, so {} can't be opened.", file.display()))?;
            std::process::Command::new(exe)
        }
    };
    cmd.arg(file);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn().map(|_| ()).map_err(|e| format!("Couldn't start {name}: {e}"))
}

fn program(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(&file)));
    if let Some(p) = here.filter(|p| p.is_file()) {
        return Some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Shows an error (started from a file manager, nobody reads stderr).
fn tell_error(e: &str) {
    if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let _ = rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("Starbase").set_description(e).show();
    }
}
